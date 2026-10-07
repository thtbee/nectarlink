// SPDX-License-Identifier: GPL-3.0-or-later
//! Phone storage in File Explorer (`docs/protocol/storage.md`, `PLAN.md` §3.6).
//!
//! Registers a Windows Cloud Files (`cfapi`) sync root per paired phone under
//! `%USERPROFILE%\Nectarlink\<Phone name>` when the phone's `storage` toggle is
//! enabled. Placeholders are populated on demand when folders are opened
//! (`CF_CALLBACK_TYPE_FETCH_PLACEHOLDERS`) and hydrated on open via ranged
//! reads (`CF_CALLBACK_TYPE_FETCH_DATA`). Renames, deletes, and new or modified
//! files dropped into the sync root are forwarded to the phone.

#![allow(unsafe_code)]

use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, c_void},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use nectarlink_core::{DeviceId, LinkState, NodeEvent, StorageEntry};
use windows::{
    Security::Cryptography::{BinaryStringEncoding, CryptographicBuffer},
    Storage::{
        Provider::{
            StorageProviderHardlinkPolicy, StorageProviderHydrationPolicy,
            StorageProviderHydrationPolicyModifier, StorageProviderInSyncPolicy,
            StorageProviderPopulationPolicy, StorageProviderSyncRootInfo, StorageProviderSyncRootManager,
        },
        StorageFolder,
    },
    Win32::{
        Foundation::{
            CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, NTSTATUS,
            STATUS_ACCESS_DENIED, STATUS_CLOUD_FILE_NETWORK_UNAVAILABLE, STATUS_CLOUD_FILE_UNSUCCESSFUL,
            STATUS_SUCCESS,
        },
        Storage::{
            CloudFilters::{
                CF_CALLBACK_INFO, CF_CALLBACK_PARAMETERS, CF_CALLBACK_REGISTRATION,
                CF_CALLBACK_TYPE_FETCH_DATA, CF_CALLBACK_TYPE_FETCH_PLACEHOLDERS, CF_CALLBACK_TYPE_NONE,
                CF_CALLBACK_TYPE_NOTIFY_DELETE, CF_CALLBACK_TYPE_NOTIFY_RENAME,
                CF_CONNECT_FLAG_REQUIRE_FULL_FILE_PATH, CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO,
                CF_CONNECTION_KEY, CF_CONVERT_FLAG_MARK_IN_SYNC, CF_FS_METADATA, CF_IN_SYNC_STATE_IN_SYNC,
                CF_OPERATION_INFO, CF_OPERATION_PARAMETERS, CF_OPERATION_PARAMETERS_0,
                CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAG_DISABLE_ON_DEMAND_POPULATION, CF_OPERATION_TYPE,
                CF_OPERATION_TYPE_ACK_DELETE, CF_OPERATION_TYPE_ACK_RENAME, CF_OPERATION_TYPE_TRANSFER_DATA,
                CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS, CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC,
                CF_PLACEHOLDER_CREATE_INFO, CF_PLACEHOLDER_STATE_IN_SYNC, CF_PLACEHOLDER_STATE_PLACEHOLDER,
                CF_SET_IN_SYNC_FLAG_NONE, CfConnectSyncRoot, CfConvertToPlaceholder, CfCreatePlaceholders,
                CfDisconnectSyncRoot, CfExecute, CfGetPlaceholderStateFromAttributeTag,
                CfReportProviderProgress, CfSetInSyncState,
            },
            FileSystem::{
                CreateFileW, FILE_ACTION_ADDED, FILE_ACTION_MODIFIED, FILE_ACTION_RENAMED_NEW_NAME,
                FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS,
                FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME,
                FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_NOTIFY_INFORMATION,
                FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FindClose, FindFirstFileW,
                OPEN_EXISTING, ReadDirectoryChangesW, WIN32_FIND_DATAW,
            },
        },
    },
    core::{HSTRING, PCWSTR},
};

use crate::{
    core_host,
    state::{Changes, CoreStatus},
    win,
};

/// Provider prefix for sync root IDs (`Nectarlink!<device_id>`).
const PROVIDER_PREFIX: &str = "Nectarlink!";

/// Difference between Windows FILETIME epoch (1601-01-01) and Unix epoch
/// (1970-01-01) in 100-nanosecond intervals.
const EPOCH_DIFF_100NS: i64 = 116_444_736_000_000_000;

/// Cloud file network unavailable NTSTATUS (`STATUS_CLOUD_FILE_NETWORK_UNAVAILABLE`).
const STATUS_CLOUD_NETWORK_UNAVAILABLE: NTSTATUS = STATUS_CLOUD_FILE_NETWORK_UNAVAILABLE;

/// Cloud file unsuccessful / cancelled operation NTSTATUS (`STATUS_CLOUD_FILE_UNSUCCESSFUL`).
const STATUS_CLOUD_UNSUCCESSFUL: NTSTATUS = STATUS_CLOUD_FILE_UNSUCCESSFUL;

/// Chunk size for hydrating files via `CF_OPERATION_TYPE_TRANSFER_DATA` (must be
/// a multiple of 4096 bytes for non-EOF chunks).
const HYDRATE_CHUNK_BYTES: usize = 256 * 1024;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct ActiveRoot {
    device: DeviceId,
    name: String,
    root_path: PathBuf,
    connection_key: CF_CONNECTION_KEY,
    stop_watcher: Arc<AtomicBool>,
    populated_dirs: Mutex<HashSet<String>>,
    uploading: Mutex<HashSet<String>>,
}

static ACTIVE_ROOTS: Mutex<Option<HashMap<DeviceId, Arc<ActiveRoot>>>> = Mutex::new(None);
static RECONCILING: Mutex<()> = Mutex::new(());

fn active_roots() -> MutexGuard<'static, Option<HashMap<DeviceId, Arc<ActiveRoot>>>> {
    lock(&ACTIVE_ROOTS)
}

fn get_active_root(device: &DeviceId) -> Option<Arc<ActiveRoot>> {
    active_roots().as_ref().and_then(|m| m.get(device).cloned())
}

/// Starts watching paired devices and their `storage` toggles to register or
/// unregister File Explorer sync roots.
pub fn start() {
    {
        let mut map = active_roots();
        if map.is_none() {
            *map = Some(HashMap::new());
        }
    }
    core_host::host().hub.subscribe(
        Changes::DEVICES | Changes::STATUS | Changes::CAPABILITIES | Changes::TOGGLES,
        || {
            sync();
            true
        },
    );
    sync();
}

/// Triggers a reconciliation of active sync roots (e.g. after a device toggle changes).
pub fn sync() {
    let host = core_host::host();
    let Some(node) = core_host::node() else { return };
    let ready = host.hub.read(|s| matches!(s.status, Some(CoreStatus::Ready { .. })));
    if !ready {
        return;
    }
    let devices = host.hub.read(|s| {
        s.devices
            .iter()
            .map(|d| (d.id, d.info.name.clone(), matches!(d.link, LinkState::Online { .. })))
            .collect::<Vec<_>>()
    });
    let mut desired: Vec<(DeviceId, String, bool)> = Vec::new();
    let mut disabled: Vec<(DeviceId, String)> = Vec::new();
    for (id, name, online) in devices {
        let enabled = node
            .device_toggles(id)
            .ok()
            .and_then(|t| t.into_iter().find(|(k, _)| *k == "storage").map(|(_, on)| on))
            .unwrap_or(false);
        if enabled {
            desired.push((id, name, online));
        } else {
            disabled.push((id, name));
        }
    }

    let _ = std::thread::Builder::new().name("storage-sync".into()).spawn(move || {
        let _guard = lock(&RECONCILING);
        win::with_com(|| reconcile(&desired, &disabled));
    });
}

/// Handles core events (`StorageChanged`, `DeviceRemoved`, `LinkChanged`).
pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::DeviceRemoved(device) => {
            let device = *device;
            let _ = std::thread::Builder::new().name("storage-unpair".into()).spawn(move || {
                let _guard = lock(&RECONCILING);
                win::with_com(|| remove_device_root(device, None));
            });
        }
        NodeEvent::DeviceAdded(_)
        | NodeEvent::PeerInfoChanged { .. }
        | NodeEvent::Capabilities(_)
        | NodeEvent::LinkChanged { .. } => {
            sync();
        }
        NodeEvent::StorageChanged { device, path } => {
            let device = *device;
            let path = path.clone();
            if let Some(root) = get_active_root(&device) {
                core_host::spawn(async move {
                    refresh_folder(root, &path).await;
                });
            }
        }
        _ => {}
    }
}

/// Disconnects active sync roots when the desktop app exits.
pub fn shutdown() {
    let roots: Vec<Arc<ActiveRoot>> =
        active_roots().as_mut().map(|m| m.drain().map(|(_, v)| v).collect()).unwrap_or_default();
    for root in roots {
        root.stop_watcher.store(true, Ordering::Relaxed);
        // SAFETY: connection_key was obtained from CfConnectSyncRoot.
        let _ = unsafe { CfDisconnectSyncRoot(root.connection_key) };
    }
}

/// Unregisters all `Nectarlink!*` sync roots from Windows (`--uninstall`).
pub fn unregister_all() {
    shutdown();
    win::with_com(|| {
        if let Ok(roots) = StorageProviderSyncRootManager::GetCurrentSyncRoots() {
            let count = roots.Size().unwrap_or(0);
            for i in 0..count {
                if let Ok(info) = roots.GetAt(i)
                    && let Ok(id) = info.Id()
                {
                    let id_str = id.to_string_lossy();
                    if id_str.starts_with(PROVIDER_PREFIX) {
                        let folder_path = info
                            .Path()
                            .ok()
                            .and_then(|f| f.Path().ok())
                            .map(|p| PathBuf::from(p.to_string_lossy()));
                        let _ = StorageProviderSyncRootManager::Unregister(&id);
                        if let Some(path) = folder_path {
                            cleanup_sync_root_dir(&path);
                        }
                    }
                }
            }
        }
        if let Some(base) = base_sync_dir() {
            let _ = std::fs::remove_dir(&base);
        }
    });
}

/// Returns the local sync root path for a paired phone (`%USERPROFILE%\Nectarlink\<Phone name>`).
pub fn sync_root_path_for(device: DeviceId) -> Option<PathBuf> {
    if let Some(active) = get_active_root(&device) {
        return Some(active.root_path.clone());
    }
    let name = core_host::host().hub.read(|s| s.name_of(&device))?;
    Some(base_sync_dir()?.join(sanitize_folder_name(&name)))
}

/// Opens a paired phone's sync root folder in File Explorer.
pub fn open_in_explorer(device: DeviceId) -> Result<(), String> {
    let path =
        sync_root_path_for(device).ok_or_else(|| "Phone storage folder is not available".to_owned())?;
    if !path.exists() {
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    }
    win::shell::run(Path::new("explorer.exe"), &format!("\"{}\"", path.display()))
}

fn base_sync_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join("Nectarlink"))
}

fn sync_root_id(device: DeviceId) -> HSTRING {
    HSTRING::from(format!("{PROVIDER_PREFIX}{device}"))
}

/// Sanitizes a phone's display name into a valid Windows directory name.
pub fn sanitize_folder_name(device_name: &str) -> String {
    let cleaned: String = device_name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { ' ' } else { c })
        .collect();
    let mut name = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    name = name.trim_end_matches(['.', ' ']).chars().take(64).collect();
    let stem = name.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if name.is_empty() {
        "Phone".into()
    } else if reserved {
        format!("{name} phone")
    } else {
        name
    }
}

/// Converts a Unix timestamp in milliseconds to Windows FILETIME 100ns ticks
/// since 1601-01-01.
pub fn unix_ms_to_filetime_ticks(unix_ms: i64) -> i64 {
    EPOCH_DIFF_100NS.saturating_add(unix_ms.max(0).saturating_mul(10_000))
}

/// Converts Windows FILETIME 100ns ticks since 1601-01-01 to Unix milliseconds.
#[cfg(test)]
pub fn filetime_ticks_to_unix_ms(ticks: i64) -> i64 {
    ticks.saturating_sub(EPOCH_DIFF_100NS).max(0) / 10_000
}

/// Computes the slash-separated relative path inside `root_path` from a full
/// local path (`""` for the sync root itself). Returns `None` if `full_path` is
/// not inside `root_path`.
pub fn relative_path_under_root(root_path: &Path, full_path: &Path) -> Option<String> {
    let root_norm = normalize_win_path(root_path);
    let full_norm = normalize_win_path(full_path);
    let root_lower = root_norm.to_lowercase();
    let full_lower = full_norm.to_lowercase();
    if full_lower == root_lower {
        return Some(String::new());
    }
    let prefix = format!("{root_lower}\\");
    if !full_lower.starts_with(&prefix) {
        return None;
    }
    let rel = &full_norm[prefix.len()..];
    let parts: Vec<&str> = rel.split('\\').filter(|s| !s.is_empty()).collect();
    if parts.iter().any(|p| *p == "." || *p == "..") {
        return None;
    }
    Some(parts.join("/"))
}

fn normalize_win_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    let stripped = s.strip_prefix(r"\\?\").unwrap_or(&s);
    stripped.trim_end_matches('\\').to_owned()
}

fn reconcile(desired: &[(DeviceId, String, bool)], disabled: &[(DeviceId, String)]) {
    // Unregister any disabled devices that belong to this instance.
    for (id, name) in disabled {
        remove_device_root(*id, Some(name));
    }

    // Remove any active roots no longer in `desired`.
    let active_ids: Vec<DeviceId> =
        active_roots().as_ref().map(|m| m.keys().copied().collect()).unwrap_or_default();
    for id in active_ids {
        if !desired.iter().any(|(d, _, _)| *d == id) {
            remove_device_root(id, None);
        }
    }

    // Register and connect desired roots.
    for (id, name, online) in desired {
        let already = get_active_root(id);
        if let Some(existing) = already {
            if existing.name == *name {
                if *online {
                    let root = existing.clone();
                    core_host::spawn(async move {
                        let is_root_populated = lock(&root.populated_dirs).contains("");
                        if is_root_populated {
                            refresh_folder(root, "").await;
                        }
                    });
                }
                continue;
            }
            // Name changed: replace root.
            remove_device_root(*id, Some(&existing.name));
        }

        if let Err(e) = register_and_connect(*id, name, *online) {
            tracing::warn!(device = %id.short(), error = %e, "can't register phone storage sync root");
        }
    }
}

fn remove_device_root(device: DeviceId, known_name: Option<&str>) {
    let removed = active_roots().as_mut().and_then(|m| m.remove(&device));
    let mut folder_to_clean: Option<PathBuf> = None;
    if let Some(root) = removed {
        root.stop_watcher.store(true, Ordering::Relaxed);
        // SAFETY: connection_key was obtained from CfConnectSyncRoot.
        let _ = unsafe { CfDisconnectSyncRoot(root.connection_key) };
        folder_to_clean = Some(root.root_path.clone());
    } else if let Some(name) = known_name
        && let Some(base) = base_sync_dir()
    {
        folder_to_clean = Some(base.join(sanitize_folder_name(name)));
    }

    let id = sync_root_id(device);
    let _ = StorageProviderSyncRootManager::Unregister(&id);

    if let Some(path) = folder_to_clean {
        cleanup_sync_root_dir(&path);
    }
    if let Some(base) = base_sync_dir() {
        let _ = std::fs::remove_dir(&base);
    }
}

fn cleanup_sync_root_dir(path: &Path) {
    if !path.exists() {
        return;
    }
    let _ = std::fs::remove_dir_all(path);
}

fn register_and_connect(device: DeviceId, name: &str, online: bool) -> Result<(), String> {
    let base = base_sync_dir().ok_or_else(|| "no user home directory".to_owned())?;
    let folder_name = sanitize_folder_name(name);
    let root_path = base.join(&folder_name);
    std::fs::create_dir_all(&root_path).map_err(|e| format!("create sync root dir: {e}"))?;

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let icon_resource = format!("{},0", exe.display());

    let info = StorageProviderSyncRootInfo::new().map_err(|e| format!("SyncRootInfo::new: {e}"))?;
    info.SetId(&sync_root_id(device)).map_err(|e| format!("SetId: {e}"))?;

    let folder = StorageFolder::GetFolderFromPathAsync(&HSTRING::from(root_path.as_os_str()))
        .map_err(|e| format!("GetFolderFromPathAsync: {e}"))?
        .join()
        .map_err(|e| format!("GetFolderFromPathAsync.join: {e}"))?;
    info.SetPath(&folder).map_err(|e| format!("SetPath: {e}"))?;
    info.SetDisplayNameResource(&HSTRING::from(name)).map_err(|e| format!("SetDisplayNameResource: {e}"))?;
    info.SetIconResource(&HSTRING::from(&icon_resource)).map_err(|e| format!("SetIconResource: {e}"))?;
    info.SetVersion(&HSTRING::from(env!("CARGO_PKG_VERSION"))).map_err(|e| format!("SetVersion: {e}"))?;
    info.SetHydrationPolicy(StorageProviderHydrationPolicy::Full)
        .map_err(|e| format!("SetHydrationPolicy: {e}"))?;
    info.SetHydrationPolicyModifier(StorageProviderHydrationPolicyModifier::None)
        .map_err(|e| format!("SetHydrationPolicyModifier: {e}"))?;
    info.SetPopulationPolicy(StorageProviderPopulationPolicy::Full)
        .map_err(|e| format!("SetPopulationPolicy: {e}"))?;
    info.SetInSyncPolicy(
        StorageProviderInSyncPolicy::FileCreationTime | StorageProviderInSyncPolicy::DirectoryCreationTime,
    )
    .map_err(|e| format!("SetInSyncPolicy: {e}"))?;
    info.SetHardlinkPolicy(StorageProviderHardlinkPolicy::None)
        .map_err(|e| format!("SetHardlinkPolicy: {e}"))?;
    info.SetShowSiblingsAsGroup(false).map_err(|e| format!("SetShowSiblingsAsGroup: {e}"))?;

    let context_buf = CryptographicBuffer::ConvertStringToBinary(
        &HSTRING::from(device.to_string()),
        BinaryStringEncoding::Utf8,
    )
    .map_err(|e| format!("ConvertStringToBinary: {e}"))?;
    info.SetContext(&context_buf).map_err(|e| format!("SetContext: {e}"))?;

    StorageProviderSyncRootManager::Register(&info).map_err(|e| format!("Register: {e}"))?;

    let callbacks = [
        CF_CALLBACK_REGISTRATION {
            Type: CF_CALLBACK_TYPE_FETCH_PLACEHOLDERS,
            Callback: Some(on_fetch_placeholders),
        },
        CF_CALLBACK_REGISTRATION { Type: CF_CALLBACK_TYPE_FETCH_DATA, Callback: Some(on_fetch_data) },
        CF_CALLBACK_REGISTRATION { Type: CF_CALLBACK_TYPE_NOTIFY_RENAME, Callback: Some(on_notify_rename) },
        CF_CALLBACK_REGISTRATION { Type: CF_CALLBACK_TYPE_NOTIFY_DELETE, Callback: Some(on_notify_delete) },
        CF_CALLBACK_REGISTRATION { Type: CF_CALLBACK_TYPE_NONE, Callback: None },
    ];

    // Keep the DeviceId in a heap allocation for the process lifetime so async
    // cfapi callbacks always read a valid DeviceId and look up `ACTIVE_ROOTS`.
    let ctx_ptr: *const c_void = Box::into_raw(Box::new(device)).cast();
    let wide_root = to_wide_null(&root_path);
    // SAFETY: wide_root is null-terminated, callbacks ends with CF_CALLBACK_TYPE_NONE.
    let connection_key = unsafe {
        CfConnectSyncRoot(
            PCWSTR(wide_root.as_ptr()),
            callbacks.as_ptr(),
            Some(ctx_ptr),
            CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO | CF_CONNECT_FLAG_REQUIRE_FULL_FILE_PATH,
        )
    }
    .map_err(|e| format!("CfConnectSyncRoot: {e}"))?;

    let stop_watcher = Arc::new(AtomicBool::new(false));
    let active = Arc::new(ActiveRoot {
        device,
        name: name.to_owned(),
        root_path: root_path.clone(),
        connection_key,
        stop_watcher: stop_watcher.clone(),
        populated_dirs: Mutex::new(HashSet::new()),
        uploading: Mutex::new(HashSet::new()),
    });

    if let Some(map) = active_roots().as_mut() {
        map.insert(device, active.clone());
    }

    start_directory_watcher(active.clone(), stop_watcher);

    if online {
        core_host::spawn(async move {
            // Pre-populate top-level placeholders when connected so opening the
            // sync root in Explorer or inspecting it immediately shows folders.
            refresh_folder(active, "").await;
        });
    }

    tracing::info!(device = %device.short(), "registered phone storage sync root");
    Ok(())
}

fn to_wide_null(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

unsafe fn pcwstr_to_string(ptr: PCWSTR) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: caller guarantees `ptr` is a valid null-terminated wide string.
    let slice = unsafe { ptr.as_wide() };
    std::ffi::OsString::from_wide(slice).to_string_lossy().into_owned()
}

unsafe fn callback_root_and_path(
    info: *const CF_CALLBACK_INFO,
) -> Option<(Arc<ActiveRoot>, String, CF_CONNECTION_KEY, i64, i64)> {
    // SAFETY: Windows passes a valid CF_CALLBACK_INFO pointer to registered callbacks.
    let cb = unsafe { info.as_ref()? };
    if cb.CallbackContext.is_null() {
        return None;
    }
    // SAFETY: CallbackContext points to a leaked `Box<DeviceId>`.
    let device = unsafe { *(cb.CallbackContext as *const DeviceId) };
    let root = get_active_root(&device)?;
    // SAFETY: VolumeDosName and NormalizedPath are valid PCWSTRs in CF_CALLBACK_INFO.
    let vol = unsafe { pcwstr_to_string(cb.VolumeDosName) };
    // SAFETY: NormalizedPath is valid when CF_CONNECT_FLAG_REQUIRE_FULL_FILE_PATH is set.
    let norm = unsafe { pcwstr_to_string(cb.NormalizedPath) };
    let full = PathBuf::from(format!("{vol}{norm}"));
    let rel = relative_path_under_root(&root.root_path, &full)?;
    Some((root, rel, cb.ConnectionKey, cb.TransferKey, cb.RequestKey))
}

fn make_op_info(
    connection_key: CF_CONNECTION_KEY,
    transfer_key: i64,
    request_key: i64,
    op_type: CF_OPERATION_TYPE,
) -> CF_OPERATION_INFO {
    CF_OPERATION_INFO {
        StructSize: std::mem::size_of::<CF_OPERATION_INFO>() as u32,
        Type: op_type,
        ConnectionKey: connection_key,
        TransferKey: transfer_key,
        CorrelationVector: std::ptr::null(),
        SyncStatus: std::ptr::null(),
        RequestKey: request_key,
    }
}

unsafe extern "system" fn on_fetch_placeholders(
    callback_info: *const CF_CALLBACK_INFO,
    _callback_parameters: *const CF_CALLBACK_PARAMETERS,
) {
    // SAFETY: callback_info is provided by Windows cfapi.
    let Some((root, rel_path, connection_key, transfer_key, request_key)) =
        (unsafe { callback_root_and_path(callback_info) })
    else {
        return;
    };

    core_host::spawn(async move {
        let Some(node) = core_host::node() else {
            ack_placeholders_failed(connection_key, transfer_key, request_key);
            return;
        };
        match node.storage_list(root.device, &rel_path).await {
            Ok(entries) => {
                lock(&root.populated_dirs).insert(rel_path.clone());
                transfer_placeholders(connection_key, transfer_key, request_key, &rel_path, &entries);
            }
            Err(e) => {
                tracing::debug!(
                    device = %root.device.short(),
                    error = %e,
                    "fetch placeholders failed"
                );
                ack_placeholders_failed(connection_key, transfer_key, request_key);
            }
        }
    });
}

fn transfer_placeholders(
    connection_key: CF_CONNECTION_KEY,
    transfer_key: i64,
    request_key: i64,
    parent_rel: &str,
    entries: &[StorageEntry],
) {
    let mut wide_names: Vec<Vec<u16>> = Vec::with_capacity(entries.len());
    let mut identities: Vec<Vec<u8>> = Vec::with_capacity(entries.len());
    for entry in entries {
        wide_names.push(to_wide_null(&entry.name));
        let rel =
            if parent_rel.is_empty() { entry.name.clone() } else { format!("{parent_rel}/{}", entry.name) };
        identities.push(rel.into_bytes());
    }

    let mut create_infos: Vec<CF_PLACEHOLDER_CREATE_INFO> = entries
        .iter()
        .enumerate()
        .map(|(i, entry)| build_placeholder_create_info(entry, &wide_names[i], &identities[i]))
        .collect();

    let op_info =
        make_op_info(connection_key, transfer_key, request_key, CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS);
    let mut op_params = CF_OPERATION_PARAMETERS {
        ParamSize: std::mem::size_of::<CF_OPERATION_PARAMETERS>() as u32,
        Anonymous: CF_OPERATION_PARAMETERS_0 {
            TransferPlaceholders: windows::Win32::Storage::CloudFilters::CF_OPERATION_PARAMETERS_0_4 {
                Flags: CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAG_DISABLE_ON_DEMAND_POPULATION,
                CompletionStatus: STATUS_SUCCESS,
                PlaceholderTotalCount: create_infos.len() as i64,
                PlaceholderArray: if create_infos.is_empty() {
                    std::ptr::null_mut()
                } else {
                    create_infos.as_mut_ptr()
                },
                PlaceholderCount: create_infos.len() as u32,
                EntriesProcessed: 0,
            },
        },
    };
    // SAFETY: op_info and op_params point to valid stack structures and buffers.
    let _ = unsafe { CfExecute(&op_info, &mut op_params) };
}

fn ack_placeholders_failed(connection_key: CF_CONNECTION_KEY, transfer_key: i64, request_key: i64) {
    let op_info =
        make_op_info(connection_key, transfer_key, request_key, CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS);
    let mut op_params = CF_OPERATION_PARAMETERS {
        ParamSize: std::mem::size_of::<CF_OPERATION_PARAMETERS>() as u32,
        Anonymous: CF_OPERATION_PARAMETERS_0 {
            TransferPlaceholders: windows::Win32::Storage::CloudFilters::CF_OPERATION_PARAMETERS_0_4 {
                Flags: windows::Win32::Storage::CloudFilters::CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAGS(0),
                CompletionStatus: STATUS_CLOUD_NETWORK_UNAVAILABLE,
                PlaceholderTotalCount: 0,
                PlaceholderArray: std::ptr::null_mut(),
                PlaceholderCount: 0,
                EntriesProcessed: 0,
            },
        },
    };
    // SAFETY: op_info and op_params are valid.
    let _ = unsafe { CfExecute(&op_info, &mut op_params) };
}

fn build_placeholder_create_info(
    entry: &StorageEntry,
    wide_name: &[u16],
    identity: &[u8],
) -> CF_PLACEHOLDER_CREATE_INFO {
    let ticks = unix_ms_to_filetime_ticks(entry.modified);
    let attrs = if entry.is_dir { FILE_ATTRIBUTE_DIRECTORY.0 } else { FILE_ATTRIBUTE_NORMAL.0 };
    let file_size = if entry.is_dir { 0 } else { entry.size as i64 };
    CF_PLACEHOLDER_CREATE_INFO {
        RelativeFileName: PCWSTR(wide_name.as_ptr()),
        FsMetadata: CF_FS_METADATA {
            BasicInfo: FILE_BASIC_INFO {
                CreationTime: ticks,
                LastAccessTime: ticks,
                LastWriteTime: ticks,
                ChangeTime: ticks,
                FileAttributes: attrs,
            },
            FileSize: file_size,
        },
        FileIdentity: identity.as_ptr().cast(),
        FileIdentityLength: identity.len() as u32,
        Flags: CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC,
        Result: windows::core::HRESULT(0),
        CreateUsn: 0,
    }
}

unsafe extern "system" fn on_fetch_data(
    callback_info: *const CF_CALLBACK_INFO,
    callback_parameters: *const CF_CALLBACK_PARAMETERS,
) {
    // SAFETY: callback_info and callback_parameters are provided by Windows cfapi.
    let Some((root, rel_path, connection_key, transfer_key, request_key)) =
        (unsafe { callback_root_and_path(callback_info) })
    else {
        return;
    };
    let Some(params) = (unsafe { callback_parameters.as_ref() }) else {
        return;
    };
    // SAFETY: Union variant matches CF_CALLBACK_TYPE_FETCH_DATA.
    let fetch = unsafe { params.Anonymous.FetchData };
    let required_offset = fetch.RequiredFileOffset.max(0) as u64;
    let required_length = fetch.RequiredLength.max(0) as u64;

    core_host::spawn(async move {
        let Some(node) = core_host::node() else {
            ack_data_failed(
                connection_key,
                transfer_key,
                request_key,
                required_offset as i64,
                STATUS_CLOUD_NETWORK_UNAVAILABLE,
            );
            return;
        };

        let (meta, mut recv) = match node
            .storage_read_stream(root.device, &rel_path, required_offset, Some(required_length))
            .await
        {
            Ok(res) => res,
            Err(e) => {
                tracing::debug!(
                    device = %root.device.short(),
                    error = %e,
                    "fetch data stream failed"
                );
                ack_data_failed(
                    connection_key,
                    transfer_key,
                    request_key,
                    required_offset as i64,
                    STATUS_CLOUD_NETWORK_UNAVAILABLE,
                );
                return;
            }
        };
        let total_size = meta.size;

        if required_length == 0 {
            let _ = transfer_data_chunk(
                connection_key,
                transfer_key,
                request_key,
                &[],
                required_offset as i64,
                STATUS_SUCCESS,
            );
            return;
        }

        let mut current_offset = required_offset;
        let mut remaining = required_length;
        let mut buf = Vec::with_capacity(HYDRATE_CHUNK_BYTES);

        while remaining > 0 {
            let want = (remaining as usize).min(HYDRATE_CHUNK_BYTES);
            buf.clear();
            while buf.len() < want {
                match recv.read_chunk(want - buf.len()).await {
                    Ok(Some(chunk)) => buf.extend_from_slice(&chunk),
                    Ok(None) => break,
                    Err(e) => {
                        tracing::debug!(error = %e, "hydration stream read error");
                        ack_data_failed(
                            connection_key,
                            transfer_key,
                            request_key,
                            current_offset as i64,
                            STATUS_CLOUD_NETWORK_UNAVAILABLE,
                        );
                        return;
                    }
                }
            }
            if buf.is_empty() {
                break;
            }
            let len = buf.len() as u64;
            let completed = (current_offset + len).min(total_size);
            // SAFETY: valid connection_key and transfer_key during active fetch.
            let _ = unsafe {
                CfReportProviderProgress(connection_key, transfer_key, total_size as i64, completed as i64)
            };
            if !transfer_data_chunk(
                connection_key,
                transfer_key,
                request_key,
                &buf,
                current_offset as i64,
                STATUS_SUCCESS,
            ) {
                return;
            }
            current_offset += len;
            remaining = remaining.saturating_sub(len);
        }
    });
}

fn transfer_data_chunk(
    connection_key: CF_CONNECTION_KEY,
    transfer_key: i64,
    request_key: i64,
    data: &[u8],
    offset: i64,
    status: NTSTATUS,
) -> bool {
    let op_info = make_op_info(connection_key, transfer_key, request_key, CF_OPERATION_TYPE_TRANSFER_DATA);
    let mut op_params = CF_OPERATION_PARAMETERS {
        ParamSize: std::mem::size_of::<CF_OPERATION_PARAMETERS>() as u32,
        Anonymous: CF_OPERATION_PARAMETERS_0 {
            TransferData: windows::Win32::Storage::CloudFilters::CF_OPERATION_PARAMETERS_0_0 {
                Flags: windows::Win32::Storage::CloudFilters::CF_OPERATION_TRANSFER_DATA_FLAGS(0),
                CompletionStatus: status,
                Buffer: if data.is_empty() { std::ptr::null() } else { data.as_ptr().cast() },
                Offset: offset,
                Length: data.len() as i64,
            },
        },
    };
    // SAFETY: op_info and op_params reference valid memory for the duration of CfExecute.
    unsafe { CfExecute(&op_info, &mut op_params) }.is_ok()
}

fn ack_data_failed(
    connection_key: CF_CONNECTION_KEY,
    transfer_key: i64,
    request_key: i64,
    offset: i64,
    status: NTSTATUS,
) {
    let _ = transfer_data_chunk(connection_key, transfer_key, request_key, &[], offset, status);
}

unsafe extern "system" fn on_notify_rename(
    callback_info: *const CF_CALLBACK_INFO,
    callback_parameters: *const CF_CALLBACK_PARAMETERS,
) {
    // SAFETY: callback_info and callback_parameters are provided by Windows cfapi.
    let Some((root, old_rel, connection_key, transfer_key, request_key)) =
        (unsafe { callback_root_and_path(callback_info) })
    else {
        return;
    };
    let Some(cb) = (unsafe { callback_info.as_ref() }) else { return };
    let Some(params) = (unsafe { callback_parameters.as_ref() }) else { return };
    // SAFETY: Union variant matches CF_CALLBACK_TYPE_NOTIFY_RENAME.
    let rename = unsafe { params.Anonymous.Rename };
    // SAFETY: VolumeDosName and TargetPath are valid PCWSTRs.
    let vol = unsafe { pcwstr_to_string(cb.VolumeDosName) };
    let target_norm = unsafe { pcwstr_to_string(rename.TargetPath) };
    let target_full = PathBuf::from(format!("{vol}{target_norm}"));
    let new_rel = relative_path_under_root(&root.root_path, &target_full);

    core_host::spawn(async move {
        let Some(node) = core_host::node() else {
            ack_rename(connection_key, transfer_key, request_key, STATUS_CLOUD_NETWORK_UNAVAILABLE);
            return;
        };
        let status = match new_rel {
            Some(new_rel) if !old_rel.is_empty() && !new_rel.is_empty() => {
                match node.storage_rename(root.device, &old_rel, &new_rel).await {
                    Ok(()) => STATUS_SUCCESS,
                    Err(nectarlink_core::Error::Offline) => STATUS_CLOUD_NETWORK_UNAVAILABLE,
                    Err(nectarlink_core::Error::Denied) => STATUS_ACCESS_DENIED,
                    Err(_) => STATUS_CLOUD_UNSUCCESSFUL,
                }
            }
            _ => STATUS_ACCESS_DENIED,
        };
        ack_rename(connection_key, transfer_key, request_key, status);
    });
}

fn ack_rename(connection_key: CF_CONNECTION_KEY, transfer_key: i64, request_key: i64, status: NTSTATUS) {
    let op_info = make_op_info(connection_key, transfer_key, request_key, CF_OPERATION_TYPE_ACK_RENAME);
    let mut op_params = CF_OPERATION_PARAMETERS {
        ParamSize: std::mem::size_of::<CF_OPERATION_PARAMETERS>() as u32,
        Anonymous: CF_OPERATION_PARAMETERS_0 {
            AckRename: windows::Win32::Storage::CloudFilters::CF_OPERATION_PARAMETERS_0_6 {
                Flags: windows::Win32::Storage::CloudFilters::CF_OPERATION_ACK_RENAME_FLAGS(0),
                CompletionStatus: status,
            },
        },
    };
    // SAFETY: op_info and op_params are valid.
    let _ = unsafe { CfExecute(&op_info, &mut op_params) };
}

unsafe extern "system" fn on_notify_delete(
    callback_info: *const CF_CALLBACK_INFO,
    _callback_parameters: *const CF_CALLBACK_PARAMETERS,
) {
    // SAFETY: callback_info is provided by Windows cfapi.
    let Some((root, rel_path, connection_key, transfer_key, request_key)) =
        (unsafe { callback_root_and_path(callback_info) })
    else {
        return;
    };

    core_host::spawn(async move {
        if rel_path.is_empty() {
            ack_delete(connection_key, transfer_key, request_key, STATUS_ACCESS_DENIED);
            return;
        }
        let Some(node) = core_host::node() else {
            ack_delete(connection_key, transfer_key, request_key, STATUS_CLOUD_NETWORK_UNAVAILABLE);
            return;
        };
        let status = match node.storage_delete(root.device, &rel_path, true).await {
            Ok(()) => STATUS_SUCCESS,
            Err(nectarlink_core::Error::Offline) => STATUS_CLOUD_NETWORK_UNAVAILABLE,
            Err(nectarlink_core::Error::Denied) => STATUS_ACCESS_DENIED,
            Err(nectarlink_core::Error::Storage(msg)) if msg.contains("not_found") => STATUS_SUCCESS,
            Err(_) => STATUS_CLOUD_UNSUCCESSFUL,
        };
        ack_delete(connection_key, transfer_key, request_key, status);
    });
}

fn ack_delete(connection_key: CF_CONNECTION_KEY, transfer_key: i64, request_key: i64, status: NTSTATUS) {
    let op_info = make_op_info(connection_key, transfer_key, request_key, CF_OPERATION_TYPE_ACK_DELETE);
    let mut op_params = CF_OPERATION_PARAMETERS {
        ParamSize: std::mem::size_of::<CF_OPERATION_PARAMETERS>() as u32,
        Anonymous: CF_OPERATION_PARAMETERS_0 {
            AckDelete: windows::Win32::Storage::CloudFilters::CF_OPERATION_PARAMETERS_0_7 {
                Flags: windows::Win32::Storage::CloudFilters::CF_OPERATION_ACK_DELETE_FLAGS(0),
                CompletionStatus: status,
            },
        },
    };
    // SAFETY: op_info and op_params are valid.
    let _ = unsafe { CfExecute(&op_info, &mut op_params) };
}

/// Refreshes a folder's placeholders from the phone (`storage.changed` or initial connect).
async fn refresh_folder(root: Arc<ActiveRoot>, rel_folder: &str) {
    let Some(node) = core_host::node() else { return };
    let Ok(entries) = node.storage_list(root.device, rel_folder).await else { return };

    let local_dir = if rel_folder.is_empty() {
        root.root_path.clone()
    } else {
        root.root_path.join(rel_folder.replace('/', "\\"))
    };
    if !local_dir.exists() {
        return;
    }

    lock(&root.populated_dirs).insert(rel_folder.to_owned());

    let mut missing: Vec<StorageEntry> = Vec::new();
    for entry in entries {
        let child_path = local_dir.join(&entry.name);
        if !child_path.exists() {
            missing.push(entry);
        }
    }
    if missing.is_empty() {
        return;
    }

    let mut wide_names: Vec<Vec<u16>> = Vec::with_capacity(missing.len());
    let mut identities: Vec<Vec<u8>> = Vec::with_capacity(missing.len());
    for entry in &missing {
        wide_names.push(to_wide_null(&entry.name));
        let rel =
            if rel_folder.is_empty() { entry.name.clone() } else { format!("{rel_folder}/{}", entry.name) };
        identities.push(rel.into_bytes());
    }
    let mut create_infos: Vec<CF_PLACEHOLDER_CREATE_INFO> = missing
        .iter()
        .enumerate()
        .map(|(i, entry)| build_placeholder_create_info(entry, &wide_names[i], &identities[i]))
        .collect();

    let wide_dir = to_wide_null(&local_dir);
    let mut processed = 0u32;
    // SAFETY: wide_dir is null-terminated and create_infos stays alive for the call.
    let _ = unsafe {
        CfCreatePlaceholders(
            PCWSTR(wide_dir.as_ptr()),
            &mut create_infos,
            windows::Win32::Storage::CloudFilters::CF_CREATE_FLAG_NONE,
            Some(&mut processed),
        )
    };
}

/// Watches the sync root directory for files or folders dropped in by the user
/// and uploads them to the phone via `storage.write` / `storage.mkdir`.
fn start_directory_watcher(root: Arc<ActiveRoot>, stop: Arc<AtomicBool>) {
    let _ = std::thread::Builder::new().name("storage-watch".into()).spawn(move || {
        let wide_dir = to_wide_null(&root.root_path);
        // SAFETY: wide_dir is a valid null-terminated path.
        let dir_handle = unsafe {
            CreateFileW(
                PCWSTR(wide_dir.as_ptr()),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                None,
            )
        };
        let Ok(dir_handle) = dir_handle else { return };
        if dir_handle == INVALID_HANDLE_VALUE {
            return;
        }

        let mut buf = vec![0u64; 4096];
        while !stop.load(Ordering::Relaxed) {
            let mut bytes_returned = 0u32;
            // SAFETY: buf is 8-byte aligned and valid for ReadDirectoryChangesW.
            let ok = unsafe {
                ReadDirectoryChangesW(
                    dir_handle,
                    buf.as_mut_ptr().cast(),
                    (buf.len() * std::mem::size_of::<u64>()) as u32,
                    true,
                    FILE_NOTIFY_CHANGE_FILE_NAME
                        | FILE_NOTIFY_CHANGE_DIR_NAME
                        | FILE_NOTIFY_CHANGE_LAST_WRITE
                        | FILE_NOTIFY_CHANGE_SIZE,
                    Some(&mut bytes_returned),
                    None,
                    None,
                )
            }
            .is_ok();

            if !ok || stop.load(Ordering::Relaxed) || bytes_returned == 0 {
                break;
            }

            let changed_paths = parse_notify_buffer(
                // SAFETY: bytes_returned bytes were initialized by ReadDirectoryChangesW.
                unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), bytes_returned as usize) },
            );

            for rel_win in changed_paths {
                let rel_slash = rel_win.replace('\\', "/");
                if rel_slash.is_empty()
                    || rel_slash.split('/').any(|seg| seg.starts_with('.') || seg.starts_with("~$"))
                {
                    continue;
                }
                let full_path = root.root_path.join(&rel_win);
                if !needs_upload(&full_path) {
                    continue;
                }
                {
                    let mut uploading = lock(&root.uploading);
                    if !uploading.insert(rel_slash.clone()) {
                        continue;
                    }
                }
                let root_clone = root.clone();
                core_host::spawn(async move {
                    // Brief debounce so Explorer finishes writing the dropped file.
                    tokio::time::sleep(Duration::from_millis(350)).await;
                    upload_local_item(&root_clone, &rel_slash, &full_path).await;
                    lock(&root_clone.uploading).remove(&rel_slash);
                });
            }
        }

        // SAFETY: dir_handle was opened with CreateFileW above.
        let _ = unsafe { CloseHandle(dir_handle) };
    });
}

fn parse_notify_buffer(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    while offset + std::mem::size_of::<FILE_NOTIFY_INFORMATION>() <= bytes.len() {
        // SAFETY: offset is within bounds and 4-byte aligned in ReadDirectoryChangesW buffer.
        let info = unsafe { &*(bytes.as_ptr().add(offset) as *const FILE_NOTIFY_INFORMATION) };
        let name_bytes = info.FileNameLength as usize;
        let name_offset = offset + std::mem::offset_of!(FILE_NOTIFY_INFORMATION, FileName);
        if name_offset + name_bytes <= bytes.len()
            && matches!(info.Action, FILE_ACTION_ADDED | FILE_ACTION_MODIFIED | FILE_ACTION_RENAMED_NEW_NAME)
        {
            // SAFETY: FileName is a UTF-16 slice of `name_bytes / 2` u16 elements.
            let wide = unsafe {
                std::slice::from_raw_parts(bytes.as_ptr().add(name_offset) as *const u16, name_bytes / 2)
            };
            let rel = std::ffi::OsString::from_wide(wide).to_string_lossy().into_owned();
            if !out.contains(&rel) {
                out.push(rel);
            }
        }
        if info.NextEntryOffset == 0 {
            break;
        }
        offset = offset.saturating_add(info.NextEntryOffset as usize);
    }
    out
}

/// Checks whether `full_path` is a newly dropped non-placeholder file/folder or
/// a modified placeholder that is no longer in sync.
fn needs_upload(full_path: &Path) -> bool {
    let wide = to_wide_null(full_path);
    let mut find_data = WIN32_FIND_DATAW::default();
    // SAFETY: wide is null-terminated and find_data is a valid struct.
    let handle = unsafe { FindFirstFileW(PCWSTR(wide.as_ptr()), &mut find_data) };
    let Ok(handle) = handle else { return false };
    if handle == INVALID_HANDLE_VALUE {
        return false;
    }
    // SAFETY: handle came from FindFirstFileW.
    let _ = unsafe { FindClose(handle) };
    // SAFETY: dwFileAttributes and dwReserved0 (reparse tag) were populated by FindFirstFileW.
    let state =
        unsafe { CfGetPlaceholderStateFromAttributeTag(find_data.dwFileAttributes, find_data.dwReserved0) };
    if (state.0 & CF_PLACEHOLDER_STATE_PLACEHOLDER.0) == 0 {
        return true;
    }
    (state.0 & CF_PLACEHOLDER_STATE_IN_SYNC.0) == 0
}

async fn upload_local_item(root: &Arc<ActiveRoot>, rel_slash: &str, full_path: &Path) {
    if !full_path.exists() || !needs_upload(full_path) {
        return;
    }
    let Some(node) = core_host::node() else { return };
    if full_path.is_dir() {
        if node.storage_mkdir(root.device, rel_slash).await.is_ok() {
            mark_path_in_sync(full_path, rel_slash, true);
        }
    } else if full_path.is_file() && node.storage_write(root.device, rel_slash, full_path).await.is_ok() {
        mark_path_in_sync(full_path, rel_slash, false);
    }
}

fn mark_path_in_sync(full_path: &Path, rel_slash: &str, is_dir: bool) {
    let wide = to_wide_null(full_path);
    let flags = if is_dir { FILE_FLAG_BACKUP_SEMANTICS } else { Default::default() };
    // SAFETY: wide is a valid null-terminated path.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            flags,
            None,
        )
    };
    let Ok(handle): Result<HANDLE, _> = handle else { return };
    if handle == INVALID_HANDLE_VALUE {
        return;
    }
    let identity = rel_slash.as_bytes();
    // SAFETY: handle is open with read/write access; identity is valid for the call.
    let converted = unsafe {
        CfConvertToPlaceholder(
            handle,
            Some(identity.as_ptr().cast()),
            identity.len() as u32,
            CF_CONVERT_FLAG_MARK_IN_SYNC,
            None,
            None,
        )
    };
    if converted.is_err() {
        // Already a placeholder: mark it in sync.
        // SAFETY: handle is valid.
        let _ = unsafe { CfSetInSyncState(handle, CF_IN_SYNC_STATE_IN_SYNC, CF_SET_IN_SYNC_FLAG_NONE, None) };
    }
    // SAFETY: handle was opened with CreateFileW above.
    let _ = unsafe { CloseHandle(handle) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_unix_ms_to_filetime_and_back() {
        let unix_ms = 1_710_000_000_123i64;
        let ticks = unix_ms_to_filetime_ticks(unix_ms);
        assert_eq!(ticks, EPOCH_DIFF_100NS + unix_ms * 10_000);
        assert_eq!(filetime_ticks_to_unix_ms(ticks), unix_ms);
        assert_eq!(unix_ms_to_filetime_ticks(-500), EPOCH_DIFF_100NS);
    }

    #[test]
    fn builds_placeholder_metadata_for_files_and_folders() {
        let file_entry = StorageEntry {
            name: "photo.jpg".into(),
            size: 42_000,
            modified: 1_700_000_000_000,
            is_dir: false,
        };
        let wide_file = to_wide_null(&file_entry.name);
        let id_file = b"DCIM/photo.jpg";
        let info_file = build_placeholder_create_info(&file_entry, &wide_file, id_file);
        assert_eq!(info_file.FsMetadata.FileSize, 42_000);
        assert_eq!(info_file.FsMetadata.BasicInfo.FileAttributes, FILE_ATTRIBUTE_NORMAL.0);
        assert_eq!(
            info_file.FsMetadata.BasicInfo.LastWriteTime,
            unix_ms_to_filetime_ticks(1_700_000_000_000)
        );
        assert_eq!(info_file.FileIdentityLength, id_file.len() as u32);

        let dir_entry =
            StorageEntry { name: "Camera".into(), size: 999, modified: 1_700_000_000_000, is_dir: true };
        let wide_dir = to_wide_null(&dir_entry.name);
        let id_dir = b"DCIM/Camera";
        let info_dir = build_placeholder_create_info(&dir_entry, &wide_dir, id_dir);
        assert_eq!(info_dir.FsMetadata.FileSize, 0);
        assert_eq!(info_dir.FsMetadata.BasicInfo.FileAttributes, FILE_ATTRIBUTE_DIRECTORY.0);
    }

    #[test]
    fn normalizes_relative_paths_and_sanitizes_folder_names() {
        let root = Path::new(r"C:\Users\alice\Nectarlink\Pixel 9");
        assert_eq!(relative_path_under_root(root, root), Some(String::new()));
        assert_eq!(
            relative_path_under_root(root, Path::new(r"C:\Users\alice\Nectarlink\Pixel 9\DCIM\Camera\a.jpg")),
            Some("DCIM/Camera/a.jpg".into())
        );
        assert_eq!(
            relative_path_under_root(root, Path::new(r"C:\Users\alice\Nectarlink\Pixel 9 Pro\DCIM")),
            None
        );

        assert_eq!(sanitize_folder_name("Pixel 9 (test)"), "Pixel 9 (test)");
        assert_eq!(sanitize_folder_name("My:Phone*1?"), "My Phone 1");
        assert_eq!(sanitize_folder_name("CON"), "CON phone");
    }
}
