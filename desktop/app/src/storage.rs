// SPDX-License-Identifier: GPL-3.0-or-later
//! Phone storage in File Explorer (`docs/protocol/storage.md`, `PLAN.md` §3.6).
//!
//! Registers a Windows Cloud Files (`cfapi`) sync root per paired phone under
//! `%USERPROFILE%\Nectarlink\<Phone name>` when the phone's `storage` toggle is
//! enabled. Placeholders are populated on demand when folders are opened
//! (`CF_CALLBACK_TYPE_FETCH_PLACEHOLDERS`) and hydrated on open via ranged
//! reads (`CF_CALLBACK_TYPE_FETCH_DATA`). Renames, deletes, and new or modified
//! files dropped into the sync root are forwarded to the phone.

#[cfg(any(windows, target_os = "linux"))]
mod imp {

    #![allow(unsafe_code)]

    use std::path::{Path, PathBuf};
    #[cfg(windows)]
    use std::{
        collections::{HashMap, HashSet},
        ffi::{OsStr, c_void},
        os::windows::ffi::{OsStrExt, OsStringExt},
        sync::{
            Arc, Mutex, MutexGuard,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use nectarlink_core::{DeviceId, NodeEvent};
    #[cfg(windows)]
    use nectarlink_core::{LinkState, StorageEntry};
    #[cfg(windows)]
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
                    CF_CALLBACK_DELETE_FLAG_IS_UNDELETE, CF_CALLBACK_INFO, CF_CALLBACK_PARAMETERS,
                    CF_CALLBACK_REGISTRATION, CF_CALLBACK_RENAME_FLAG_IS_DIRECTORY,
                    CF_CALLBACK_RENAME_FLAG_SOURCE_IN_SCOPE, CF_CALLBACK_TYPE_FETCH_DATA,
                    CF_CALLBACK_TYPE_FETCH_PLACEHOLDERS, CF_CALLBACK_TYPE_NONE,
                    CF_CALLBACK_TYPE_NOTIFY_DELETE, CF_CALLBACK_TYPE_NOTIFY_DELETE_COMPLETION,
                    CF_CALLBACK_TYPE_NOTIFY_RENAME, CF_CONNECT_FLAG_REQUIRE_FULL_FILE_PATH,
                    CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO, CF_CONNECTION_KEY, CF_CONVERT_FLAG_MARK_IN_SYNC,
                    CF_FS_METADATA, CF_IN_SYNC_STATE_IN_SYNC, CF_OPERATION_INFO, CF_OPERATION_PARAMETERS,
                    CF_OPERATION_PARAMETERS_0,
                    CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAG_DISABLE_ON_DEMAND_POPULATION, CF_OPERATION_TYPE,
                    CF_OPERATION_TYPE_ACK_DELETE, CF_OPERATION_TYPE_ACK_RENAME,
                    CF_OPERATION_TYPE_TRANSFER_DATA, CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS,
                    CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC, CF_PLACEHOLDER_CREATE_INFO,
                    CF_PLACEHOLDER_STATE, CF_PLACEHOLDER_STATE_IN_SYNC, CF_PLACEHOLDER_STATE_PARTIAL,
                    CF_PLACEHOLDER_STATE_PLACEHOLDER, CF_SET_IN_SYNC_FLAG_NONE, CfConnectSyncRoot,
                    CfConvertToPlaceholder, CfCreatePlaceholders, CfDisconnectSyncRoot, CfExecute,
                    CfGetPlaceholderStateFromAttributeTag, CfReportProviderProgress, CfSetInSyncState,
                },
                FileSystem::{
                    CreateFileW, FILE_ACTION_ADDED, FILE_ACTION_MODIFIED, FILE_ACTION_RENAMED_NEW_NAME,
                    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_BASIC_INFO,
                    FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_DIR_NAME,
                    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE,
                    FILE_NOTIFY_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FindClose,
                    FindFirstFileW, OPEN_EXISTING, ReadDirectoryChangesW, WIN32_FIND_DATAW,
                },
            },
        },
        core::{HSTRING, PCWSTR},
    };

    #[cfg(windows)]
    use crate::i18n::tr;
    #[cfg(windows)]
    use crate::state::{Changes, CoreStatus};
    use crate::{core_host, win};

    /// Provider prefix for sync root IDs (`NectarLink!<device_id>`).
    #[cfg(windows)]
    const PROVIDER_PREFIX: &str = "Nectarlink!";

    /// Difference between Windows FILETIME epoch (1601-01-01) and Unix epoch
    /// (1970-01-01) in 100-nanosecond intervals.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    const EPOCH_DIFF_100NS: i64 = 116_444_736_000_000_000;

    /// Cloud file network unavailable NTSTATUS (`STATUS_CLOUD_FILE_NETWORK_UNAVAILABLE`).
    #[cfg(windows)]
    const STATUS_CLOUD_NETWORK_UNAVAILABLE: NTSTATUS = STATUS_CLOUD_FILE_NETWORK_UNAVAILABLE;

    /// Cloud file unsuccessful / cancelled operation NTSTATUS (`STATUS_CLOUD_FILE_UNSUCCESSFUL`).
    #[cfg(windows)]
    const STATUS_CLOUD_UNSUCCESSFUL: NTSTATUS = STATUS_CLOUD_FILE_UNSUCCESSFUL;

    /// Chunk size for hydrating files via `CF_OPERATION_TYPE_TRANSFER_DATA` (must be
    /// a multiple of 4096 bytes for non-EOF chunks).
    #[cfg(windows)]
    const HYDRATE_CHUNK_BYTES: usize = 256 * 1024;

    #[cfg(windows)]
    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[cfg(windows)]
    struct ActiveRoot {
        device: DeviceId,
        name: String,
        root_path: PathBuf,
        connection_key: CF_CONNECTION_KEY,
        stop_watcher: Arc<AtomicBool>,
        populated_dirs: Mutex<HashSet<String>>,
        uploading: Mutex<HashSet<String>>,
        /// Placeholders being removed because the phone no longer has them: the
        /// delete Windows reports for them mustn't reach the phone.
        removing: Mutex<HashSet<String>>,
        /// The phone was connected at the last sync: its folder is re-read
        /// only when it comes back, not on every sync.
        online: AtomicBool,
    }

    #[cfg(windows)]
    static ACTIVE_ROOTS: Mutex<Option<HashMap<DeviceId, Arc<ActiveRoot>>>> = Mutex::new(None);
    #[cfg(windows)]
    static INTERNED_DEVICES: Mutex<Option<HashMap<DeviceId, usize>>> = Mutex::new(None);
    #[cfg(windows)]
    static RECONCILING: Mutex<()> = Mutex::new(());

    /// What the next reconcile should reach, and whether one is running. A
    /// burst of syncs (a phone connecting sends several events at once) runs
    /// one reconcile, then one more with the newest state, instead of a
    /// thread each.
    #[cfg(windows)]
    static PENDING: Mutex<(bool, Option<Wanted>)> = Mutex::new((false, None));

    #[cfg(windows)]
    type Wanted = (Vec<(DeviceId, String, String, bool)>, Vec<(DeviceId, String)>);

    /// Whether each phone was connected when its link last changed. Only a
    /// change between connected and not matters here: the link also changes
    /// with every round-trip time update.
    #[cfg(windows)]
    static LINK_ONLINE: Mutex<Option<HashMap<DeviceId, bool>>> = Mutex::new(None);

    /// Phones whose folder couldn't be registered, and when to try again.
    #[cfg(windows)]
    static FAILED: Mutex<Option<HashMap<DeviceId, RetryLater>>> = Mutex::new(None);

    #[cfg(windows)]
    fn intern_device_id(device: DeviceId) -> *const c_void {
        let mut guard = lock(&INTERNED_DEVICES);
        let map = guard.get_or_insert_with(HashMap::new);
        let addr = *map.entry(device).or_insert_with(|| Box::into_raw(Box::new(device)) as usize);
        addr as *const c_void
    }

    #[cfg(windows)]
    fn active_roots() -> MutexGuard<'static, Option<HashMap<DeviceId, Arc<ActiveRoot>>>> {
        lock(&ACTIVE_ROOTS)
    }

    #[cfg(windows)]
    fn get_active_root(device: &DeviceId) -> Option<Arc<ActiveRoot>> {
        active_roots().as_ref().and_then(|m| m.get(device).cloned())
    }

    /// Starts watching paired devices and their `storage` toggles to register or
    /// unregister File Explorer sync roots.
    #[cfg(windows)]
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

    #[cfg(target_os = "linux")]
    pub fn start() {
        win::storage::start();
    }

    /// Triggers a reconciliation of active sync roots (e.g. after a device toggle changes).
    #[cfg(windows)]
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
        let folders = folder_names(devices.iter().map(|(id, name, _)| (*id, name.as_str())));
        let mut desired: Vec<(DeviceId, String, String, bool)> = Vec::new();
        let mut disabled: Vec<(DeviceId, String)> = Vec::new();
        for (id, name, online) in devices {
            // A permission that can't be read right now isn't "off": turning a
            // phone off removes its folder, so try again on the next sync.
            let Ok(toggles) = node.device_toggles(id) else {
                tracing::debug!(device = %id.short(), "can't read storage permission; skipping this sync");
                return;
            };
            let enabled = toggles.into_iter().find(|(k, _)| *k == "storage").is_some_and(|(_, on)| on);
            let folder = folders.get(&id).cloned().unwrap_or_else(|| sanitize_folder_name(&name));
            if enabled {
                desired.push((id, name, folder, online));
            } else {
                disabled.push((id, folder));
            }
        }

        {
            let mut pending = lock(&PENDING);
            pending.1 = Some((desired, disabled));
            if pending.0 {
                // The running reconcile picks this up when it's done.
                return;
            }
            pending.0 = true;
        }
        let spawned = std::thread::Builder::new().name("storage-sync".into()).spawn(|| {
            loop {
                let next = {
                    let mut pending = lock(&PENDING);
                    let next = pending.1.take();
                    if next.is_none() {
                        pending.0 = false;
                    }
                    next
                };
                let Some((desired, disabled)) = next else { return };
                let _guard = lock(&RECONCILING);
                win::with_com(|| reconcile(&desired, &disabled));
            }
        });
        if spawned.is_err() {
            lock(&PENDING).0 = false;
        }
    }

    /// Whether a link change moved `device` between connected and not, noting
    /// where it is now.
    #[cfg(windows)]
    fn online_changed(device: DeviceId, online: bool) -> bool {
        let mut links = lock(&LINK_ONLINE);
        links.get_or_insert_with(HashMap::new).insert(device, online) != Some(online)
    }

    #[cfg(target_os = "linux")]
    pub fn sync() {
        win::storage::sync();
    }

    /// Handles core events (`StorageChanged`, `DeviceRemoved`, `LinkChanged`).
    #[cfg(windows)]
    pub fn on_event(event: &NodeEvent) {
        match event {
            NodeEvent::DeviceRemoved(device) => {
                let device = *device;
                let _ = std::thread::Builder::new().name("storage-unpair".into()).spawn(move || {
                    let _guard = lock(&RECONCILING);
                    win::with_com(|| remove_device_root(device, None));
                });
            }
            NodeEvent::LinkChanged { device, link } => {
                if online_changed(*device, matches!(link, LinkState::Online { .. })) {
                    sync();
                }
            }
            NodeEvent::DeviceAdded(_) | NodeEvent::PeerInfoChanged { .. } | NodeEvent::Capabilities(_) => {
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

    #[cfg(target_os = "linux")]
    pub fn on_event(event: &NodeEvent) {
        win::storage::on_event(event);
    }

    /// Disconnects active sync roots when the desktop app exits.
    #[cfg(windows)]
    pub fn shutdown() {
        let roots: Vec<Arc<ActiveRoot>> =
            active_roots().as_mut().map(|m| m.drain().map(|(_, v)| v).collect()).unwrap_or_default();
        for root in roots {
            root.stop_watcher.store(true, Ordering::Relaxed);
            // SAFETY: connection_key was obtained from CfConnectSyncRoot.
            let _ = unsafe { CfDisconnectSyncRoot(root.connection_key) };
        }
    }

    #[cfg(target_os = "linux")]
    pub fn shutdown() {
        win::storage::shutdown();
    }

    /// Unregisters all `NectarLink!*` sync roots from Windows (`--uninstall`).
    #[cfg(windows)]
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

    #[cfg(target_os = "linux")]
    pub fn unregister_all() {
        win::storage::unregister_all();
    }

    /// Returns the local sync root path for a paired phone (`%USERPROFILE%\Nectarlink\<Phone name>`).
    pub fn sync_root_path_for(device: DeviceId) -> Option<PathBuf> {
        #[cfg(windows)]
        if let Some(active) = get_active_root(&device) {
            return Some(active.root_path.clone());
        }
        #[cfg(target_os = "linux")]
        if let Some(path) = win::storage::mounted_path_for(&device) {
            return Some(path);
        }
        let mut folders = core_host::host()
            .hub
            .read(|s| folder_names(s.devices.iter().map(|d| (d.id, d.info.name.as_str()))));
        Some(base_sync_dir()?.join(folders.remove(&device)?))
    }

    /// Each phone's folder under `Nectarlink`: its name, or its name and
    /// short ID when another paired phone has the same name, so two phones
    /// of the same model never share one folder (or remove each other's).
    pub fn folder_names<'a>(
        devices: impl Iterator<Item = (DeviceId, &'a str)>,
    ) -> std::collections::HashMap<DeviceId, String> {
        let named: Vec<(DeviceId, String)> =
            devices.map(|(id, name)| (id, sanitize_folder_name(name))).collect();
        named
            .iter()
            .map(|(id, folder)| {
                let shared = named.iter().any(|(other, f)| other != id && f.eq_ignore_ascii_case(folder));
                let folder = if shared { format!("{folder} ({})", id.short()) } else { folder.clone() };
                (*id, folder)
            })
            .collect()
    }

    /// Opens a paired phone's sync root folder in File Explorer.
    pub fn open_in_explorer(device: DeviceId) -> Result<(), String> {
        let path =
            sync_root_path_for(device).ok_or_else(|| "Phone storage folder is not available".to_owned())?;
        if !path.exists() {
            std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        }
        #[cfg(windows)]
        {
            win::shell::run(Path::new("explorer.exe"), &format!("\"{}\"", path.display()))
        }
        #[cfg(target_os = "linux")]
        {
            win::shell::open_folder(&path)
        }
    }

    pub(crate) fn base_sync_dir() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join("Nectarlink"))
    }

    #[cfg(windows)]
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
    #[cfg_attr(target_os = "linux", allow(dead_code))]
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
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    pub fn relative_path_under_root(root_path: &Path, full_path: &Path) -> Option<String> {
        let root_norm = normalize_win_path(root_path);
        let full_norm = normalize_win_path(full_path);
        // Compared name by name, ignoring case. (Cutting the path at the length
        // of a lowercased copy breaks on letters whose lowercase is longer,
        // such as the Turkish "İ" in a phone's name.)
        let root_parts: Vec<&str> = root_norm.split('\\').filter(|s| !s.is_empty()).collect();
        let full_parts: Vec<&str> = full_norm.split('\\').filter(|s| !s.is_empty()).collect();
        if full_parts.len() < root_parts.len()
            || !root_parts.iter().zip(&full_parts).all(|(a, b)| a.to_lowercase() == b.to_lowercase())
        {
            return None;
        }
        let parts = &full_parts[root_parts.len()..];
        if parts.iter().any(|p| *p == "." || *p == "..") {
            return None;
        }
        Some(parts.join("/"))
    }

    /// When to try registering a phone's folder again after it failed: a
    /// minute, then twice as long each time up to half an hour. A new name
    /// or folder is worth trying at once.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    struct RetryLater {
        name: String,
        folder: String,
        next: std::time::Instant,
        delay: std::time::Duration,
    }

    #[cfg_attr(target_os = "linux", allow(dead_code))]
    impl RetryLater {
        const FIRST: std::time::Duration = std::time::Duration::from_secs(60);
        const MOST: std::time::Duration = std::time::Duration::from_secs(30 * 60);

        fn new(name: &str, folder: &str, now: std::time::Instant) -> RetryLater {
            RetryLater {
                name: name.to_owned(),
                folder: folder.to_owned(),
                next: now + Self::FIRST,
                delay: Self::FIRST,
            }
        }

        /// Whether to leave this phone alone for now.
        fn waits(&self, name: &str, folder: &str, now: std::time::Instant) -> bool {
            self.name == name && self.folder == folder && now < self.next
        }

        fn failed_again(&mut self, name: &str, folder: &str, now: std::time::Instant) {
            if self.name != name || self.folder != folder {
                *self = RetryLater::new(name, folder, now);
                return;
            }
            self.delay = (self.delay * 2).min(Self::MOST);
            self.next = now + self.delay;
        }
    }

    #[cfg_attr(target_os = "linux", allow(dead_code))]
    fn normalize_win_path(path: &Path) -> String {
        let s = path.to_string_lossy();
        let stripped = s.strip_prefix(r"\\?\").unwrap_or(&s);
        stripped.trim_end_matches('\\').to_owned()
    }

    #[cfg(windows)]
    fn reconcile(desired: &[(DeviceId, String, String, bool)], disabled: &[(DeviceId, String)]) {
        // Unregister any disabled devices that belong to this instance.
        for (id, folder) in disabled {
            remove_device_root(*id, Some(folder));
            // Turned off: turning it on again tries right away.
            lock(&FAILED).get_or_insert_with(HashMap::new).remove(id);
        }

        // Remove any active roots no longer in `desired`.
        let active_ids: Vec<DeviceId> =
            active_roots().as_ref().map(|m| m.keys().copied().collect()).unwrap_or_default();
        for id in active_ids {
            if !desired.iter().any(|(d, _, _, _)| *d == id) {
                remove_device_root(id, None);
            }
        }

        // Register and connect desired roots.
        for (id, name, folder, online) in desired {
            let already = get_active_root(id);
            if let Some(existing) = already {
                if existing.name == *name
                    && existing.root_path.file_name() == Some(std::ffi::OsStr::new(folder))
                {
                    let was_online = existing.online.swap(*online, Ordering::Relaxed);
                    if *online && !was_online {
                        // Back after being away: what's on the phone may have changed.
                        let root = existing.clone();
                        core_host::spawn(refresh_known_folders(root));
                    }
                    continue;
                }
                // Name or folder changed: replace root.
                let old_folder = existing.root_path.file_name().map(|f| f.to_string_lossy().into_owned());
                remove_device_root(*id, old_folder.as_deref());
            }

            let now = std::time::Instant::now();
            let waiting = lock(&FAILED)
                .as_ref()
                .and_then(|failed| failed.get(id))
                .is_some_and(|retry| retry.waits(name, folder, now));
            if waiting {
                continue;
            }
            match register_and_connect(*id, name, folder, *online) {
                Ok(()) => {
                    lock(&FAILED).get_or_insert_with(HashMap::new).remove(id);
                }
                Err(e) => {
                    let mut failed = lock(&FAILED);
                    let retry = failed
                        .get_or_insert_with(HashMap::new)
                        .entry(*id)
                        .and_modify(|retry| retry.failed_again(name, folder, now))
                        .or_insert_with(|| RetryLater::new(name, folder, now));
                    tracing::warn!(
                        device = %id.short(),
                        error = %e,
                        retry_in_s = retry.delay.as_secs(),
                        "can't register phone storage sync root"
                    );
                    // Nothing else may change meanwhile: try again once the wait is over.
                    let delay = retry.delay;
                    let _ = std::thread::Builder::new().name("storage-retry".into()).spawn(move || {
                        std::thread::sleep(delay);
                        sync();
                    });
                }
            }
        }
    }

    #[cfg(windows)]
    fn remove_device_root(device: DeviceId, known_folder: Option<&str>) {
        let removed = active_roots().as_mut().and_then(|m| m.remove(&device));
        let mut folder_to_clean: Option<PathBuf> = None;
        if let Some(root) = removed {
            root.stop_watcher.store(true, Ordering::Relaxed);
            // SAFETY: connection_key was obtained from CfConnectSyncRoot.
            let _ = unsafe { CfDisconnectSyncRoot(root.connection_key) };
            folder_to_clean = Some(root.root_path.clone());
        } else if let Some(folder) = known_folder
            && let Some(base) = base_sync_dir()
        {
            folder_to_clean = Some(base.join(folder));
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

    /// Clears the phone's folder: its placeholders go, but a file that never
    /// reached the phone (copied in, or changed and not uploaded) is the
    /// user's own and stays where it is.
    #[cfg(windows)]
    fn cleanup_sync_root_dir(path: &Path) {
        fn clear(dir: &Path) -> usize {
            let mut kept = 0;
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    kept += clear(&path);
                    let _ = std::fs::remove_dir(&path);
                } else if needs_upload(&path) {
                    kept += 1;
                } else {
                    let _ = std::fs::remove_file(&path);
                }
            }
            kept
        }
        if !path.exists() {
            return;
        }
        let kept = clear(path);
        if kept > 0 {
            tracing::info!(kept, "kept files that never reached the phone in its old folder");
        } else {
            let _ = std::fs::remove_dir(path);
        }
    }

    #[cfg(windows)]
    fn register_and_connect(device: DeviceId, name: &str, folder: &str, online: bool) -> Result<(), String> {
        let base = base_sync_dir().ok_or_else(|| "no user home directory".to_owned())?;
        let root_path = base.join(folder);
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
        info.SetDisplayNameResource(&HSTRING::from(name))
            .map_err(|e| format!("SetDisplayNameResource: {e}"))?;
        info.SetIconResource(&HSTRING::from(&icon_resource)).map_err(|e| format!("SetIconResource: {e}"))?;
        info.SetVersion(&HSTRING::from(env!("CARGO_PKG_VERSION"))).map_err(|e| format!("SetVersion: {e}"))?;
        info.SetHydrationPolicy(StorageProviderHydrationPolicy::Full)
            .map_err(|e| format!("SetHydrationPolicy: {e}"))?;
        info.SetHydrationPolicyModifier(StorageProviderHydrationPolicyModifier::None)
            .map_err(|e| format!("SetHydrationPolicyModifier: {e}"))?;
        info.SetPopulationPolicy(StorageProviderPopulationPolicy::Full)
            .map_err(|e| format!("SetPopulationPolicy: {e}"))?;
        info.SetInSyncPolicy(
            StorageProviderInSyncPolicy::FileCreationTime
                | StorageProviderInSyncPolicy::DirectoryCreationTime,
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
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_NOTIFY_RENAME,
                Callback: Some(on_notify_rename),
            },
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_NOTIFY_DELETE,
                Callback: Some(on_notify_delete),
            },
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_NOTIFY_DELETE_COMPLETION,
                Callback: Some(on_notify_delete_completion),
            },
            CF_CALLBACK_REGISTRATION { Type: CF_CALLBACK_TYPE_NONE, Callback: None },
        ];

        // Keep one interned DeviceId allocation per paired device for the process
        // lifetime so async cfapi callbacks always read a valid DeviceId and
        // reconnects do not allocate a new Box each time.
        let ctx_ptr = intern_device_id(device);
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
            removing: Mutex::new(HashSet::new()),
            online: AtomicBool::new(online),
        });

        if let Some(map) = active_roots().as_mut() {
            map.insert(device, active.clone());
        }

        start_directory_watcher(active.clone(), stop_watcher);

        if online {
            core_host::spawn(async move {
                // Pre-populate top-level placeholders when connected so opening the
                // sync root in Explorer or inspecting it immediately shows folders,
                // and catch up on the folders shown before.
                refresh_known_folders(active).await;
            });
        }

        tracing::info!(device = %device.short(), "registered phone storage sync root");
        Ok(())
    }

    #[cfg(windows)]
    fn to_wide_null(s: impl AsRef<OsStr>) -> Vec<u16> {
        s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
    }

    #[cfg(windows)]
    unsafe fn pcwstr_to_string(ptr: PCWSTR) -> String {
        if ptr.is_null() {
            return String::new();
        }
        // SAFETY: caller guarantees `ptr` is a valid null-terminated wide string.
        let slice = unsafe { ptr.as_wide() };
        std::ffi::OsString::from_wide(slice).to_string_lossy().into_owned()
    }

    #[cfg(windows)]
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

    #[cfg(windows)]
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

    #[cfg(windows)]
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
                ack_placeholders_failed(
                    connection_key,
                    transfer_key,
                    request_key,
                    STATUS_CLOUD_NETWORK_UNAVAILABLE,
                );
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
                    let status = failure_status(root.device, &e);
                    ack_placeholders_failed(connection_key, transfer_key, request_key, status);
                }
            }
        });
    }

    #[cfg(windows)]
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
            let rel = if parent_rel.is_empty() {
                entry.name.clone()
            } else {
                format!("{parent_rel}/{}", entry.name)
            };
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

    /// What Explorer is told when the phone couldn't list or read something.
    /// A refusal (storage access off, or no All files access) isn't a network
    /// problem: it's "access denied", and NectarLink says what to turn on.
    #[cfg(windows)]
    fn failure_status(device: DeviceId, error: &nectarlink_core::Error) -> NTSTATUS {
        match error {
            nectarlink_core::Error::Denied | nectarlink_core::Error::Unsupported => {
                explain_refusal(device, matches!(error, nectarlink_core::Error::Unsupported));
                STATUS_ACCESS_DENIED
            }
            _ => STATUS_CLOUD_NETWORK_UNAVAILABLE,
        }
    }

    /// Says on this PC why the phone's folder won't open, at most every ten
    /// minutes per phone (Explorer asks again for every folder it shows).
    #[cfg(windows)]
    fn explain_refusal(device: DeviceId, no_all_files_access: bool) {
        static SHOWN: Mutex<Option<std::collections::HashMap<DeviceId, std::time::Instant>>> =
            Mutex::new(None);
        {
            let mut shown = lock(&SHOWN);
            let shown = shown.get_or_insert_with(Default::default);
            if shown.get(&device).is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(600)) {
                return;
            }
            shown.insert(device, std::time::Instant::now());
        }
        let name = core_host::host().hub.read(|s| s.name_of(&device)).unwrap_or_else(|| "your phone".into());
        let body = if no_all_files_access {
            tr!("Storage", "On the phone, allow All files access for NectarLink.")
        } else {
            tr!(
                "Storage",
                "On the phone, turn on Browse this phone's storage in File Explorer in NectarLink."
            )
        };
        crate::win::toast::show(crate::win::toast::Toast {
            device: crate::transfers::TOAST_GROUP.into(),
            key: format!("storage-refused-{device}"),
            title: tr!("Storage", "%1's files aren't shared with this PC", name),
            body,
            attribution: "NectarLink".into(),
            icon: None,
            image: None,
            actions: Vec::new(),
            reply: None,
            silent: false,
            progress: None,
            call: false,
        });
    }

    #[cfg(windows)]
    fn ack_placeholders_failed(
        connection_key: CF_CONNECTION_KEY,
        transfer_key: i64,
        request_key: i64,
        status: NTSTATUS,
    ) {
        let op_info =
            make_op_info(connection_key, transfer_key, request_key, CF_OPERATION_TYPE_TRANSFER_PLACEHOLDERS);
        let mut op_params = CF_OPERATION_PARAMETERS {
            ParamSize: std::mem::size_of::<CF_OPERATION_PARAMETERS>() as u32,
            Anonymous: CF_OPERATION_PARAMETERS_0 {
                TransferPlaceholders: windows::Win32::Storage::CloudFilters::CF_OPERATION_PARAMETERS_0_4 {
                    Flags: windows::Win32::Storage::CloudFilters::CF_OPERATION_TRANSFER_PLACEHOLDERS_FLAGS(0),
                    CompletionStatus: status,
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

    #[cfg(windows)]
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

    #[cfg(windows)]
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
                        failure_status(root.device, &e),
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
                    CfReportProviderProgress(
                        connection_key,
                        transfer_key,
                        total_size as i64,
                        completed as i64,
                    )
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
            // The phone's stream ended short: Explorer hears it now, instead
            // of waiting out its own timeout.
            if remaining > 0 {
                ack_data_failed(
                    connection_key,
                    transfer_key,
                    request_key,
                    current_offset as i64,
                    STATUS_CLOUD_NETWORK_UNAVAILABLE,
                );
            }
        });
    }

    #[cfg(windows)]
    fn transfer_data_chunk(
        connection_key: CF_CONNECTION_KEY,
        transfer_key: i64,
        request_key: i64,
        data: &[u8],
        offset: i64,
        status: NTSTATUS,
    ) -> bool {
        let op_info =
            make_op_info(connection_key, transfer_key, request_key, CF_OPERATION_TYPE_TRANSFER_DATA);
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

    #[cfg(windows)]
    fn ack_data_failed(
        connection_key: CF_CONNECTION_KEY,
        transfer_key: i64,
        request_key: i64,
        offset: i64,
        status: NTSTATUS,
    ) {
        let _ = transfer_data_chunk(connection_key, transfer_key, request_key, &[], offset, status);
    }

    #[cfg(windows)]
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
        let flags = rename.Flags;
        // Out of the phone's folder, like into the Recycle Bin: the PC keeps the
        // file, so it leaves the phone. Only a file that's all here, though: a
        // folder, or a file still in the cloud, would arrive empty.
        let moving_out = new_rel.is_none()
            && (flags.0 & CF_CALLBACK_RENAME_FLAG_SOURCE_IN_SCOPE.0) != 0
            && (flags.0 & CF_CALLBACK_RENAME_FLAG_IS_DIRECTORY.0) == 0
            && fully_here(&root.root_path.join(old_rel.replace('/', "\\")));

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
                        Err(e) => {
                            tracing::warn!(error = %e, "the phone couldn't move or rename a file");
                            STATUS_CLOUD_UNSUCCESSFUL
                        }
                    }
                }
                None if moving_out && !old_rel.is_empty() => {
                    delete_status(node.storage_delete(root.device, &old_rel, true).await)
                }
                _ => {
                    tracing::warn!(
                        "a folder, or a file not downloaded yet, can't be moved out of the phone's folder"
                    );
                    STATUS_ACCESS_DENIED
                }
            };
            ack_rename(connection_key, transfer_key, request_key, status);
        });
    }

    #[cfg(windows)]
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

    /// Windows is about to delete a placeholder. Nothing reaches the phone yet:
    /// a delete can still be taken back (Explorer does that when it moves a
    /// file to the Recycle Bin), so the phone deletes it once it's done, in
    /// [`on_notify_delete_completion`]. This only turns down what can't work.
    #[cfg(windows)]
    unsafe extern "system" fn on_notify_delete(
        callback_info: *const CF_CALLBACK_INFO,
        callback_parameters: *const CF_CALLBACK_PARAMETERS,
    ) {
        // SAFETY: callback_info is provided by Windows cfapi.
        let Some((root, rel_path, connection_key, transfer_key, request_key)) =
            (unsafe { callback_root_and_path(callback_info) })
        else {
            return;
        };
        // SAFETY: callback_parameters is provided by Windows cfapi; the union
        // variant matches CF_CALLBACK_TYPE_NOTIFY_DELETE.
        let undelete = unsafe { callback_parameters.as_ref() }.is_some_and(|p| {
            (unsafe { p.Anonymous.Delete }.Flags.0 & CF_CALLBACK_DELETE_FLAG_IS_UNDELETE.0) != 0
        });
        let status = if undelete || lock(&root.removing).contains(&rel_path) {
            STATUS_SUCCESS
        } else if rel_path.is_empty() {
            STATUS_ACCESS_DENIED
        } else if !root.online.load(Ordering::Relaxed) || core_host::node().is_none() {
            STATUS_CLOUD_NETWORK_UNAVAILABLE
        } else {
            STATUS_SUCCESS
        };
        ack_delete(connection_key, transfer_key, request_key, status);
    }

    /// A placeholder was deleted on the PC: delete it on the phone. If the
    /// phone can't, the folder is read again so the file comes back.
    #[cfg(windows)]
    unsafe extern "system" fn on_notify_delete_completion(
        callback_info: *const CF_CALLBACK_INFO,
        _callback_parameters: *const CF_CALLBACK_PARAMETERS,
    ) {
        // SAFETY: callback_info is provided by Windows cfapi.
        let Some((root, rel_path, ..)) = (unsafe { callback_root_and_path(callback_info) }) else {
            return;
        };
        if rel_path.is_empty() || lock(&root.removing).contains(&rel_path) {
            return;
        }
        core_host::spawn(async move {
            let Some(node) = core_host::node() else { return };
            if delete_status(node.storage_delete(root.device, &rel_path, true).await) != STATUS_SUCCESS {
                let parent = rel_path.rsplit_once('/').map_or("", |(dir, _)| dir).to_owned();
                refresh_folder(root, &parent).await;
            }
        });
    }

    /// What a delete on the phone means for Windows.
    #[cfg(windows)]
    fn delete_status(result: nectarlink_core::Result<()>) -> NTSTATUS {
        match result {
            // Already gone from the phone: deleted, as asked.
            Ok(()) | Err(nectarlink_core::Error::NotFound) => STATUS_SUCCESS,
            Err(nectarlink_core::Error::Offline) => STATUS_CLOUD_NETWORK_UNAVAILABLE,
            // With neither All files access nor a folder picked, the phone can't.
            Err(nectarlink_core::Error::Denied | nectarlink_core::Error::Unsupported) => {
                tracing::warn!("the phone doesn't let this PC delete its files");
                STATUS_ACCESS_DENIED
            }
            Err(e) => {
                tracing::warn!(error = %e, "the phone couldn't delete a file");
                STATUS_CLOUD_UNSUCCESSFUL
            }
        }
    }

    /// Whether a placeholder's data is all on the PC.
    #[cfg(windows)]
    fn fully_here(path: &Path) -> bool {
        use std::os::windows::fs::MetadataExt;
        const NOT_HERE: u32 = 0x0040_0000 | 0x0000_1000 | 0x0000_0200; // recall on access, offline, sparse
        std::fs::symlink_metadata(path).is_ok_and(|m| m.file_attributes() & NOT_HERE == 0)
    }

    #[cfg(windows)]
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

    /// Reads again every folder the PC already shows, top first: what changed
    /// on the phone while it was away, or before the app started, only shows
    /// once a folder is read. Folders never opened are empty here and skipped.
    #[cfg(windows)]
    async fn refresh_known_folders(root: Arc<ActiveRoot>) {
        const MOST_FOLDERS: usize = 256;
        let mut queue = std::collections::VecDeque::from([String::new()]);
        let mut done = 0;
        while let Some(rel) = queue.pop_front() {
            if done == MOST_FOLDERS {
                break;
            }
            done += 1;
            refresh_folder(root.clone(), &rel).await;
            let local = if rel.is_empty() {
                root.root_path.clone()
            } else {
                root.root_path.join(rel.replace('/', "\\"))
            };
            let Ok(children) = std::fs::read_dir(&local) else { continue };
            for child in children.flatten() {
                let path = child.path();
                // Only folders already read: listing one never opened would
                // fetch it, and so on through the whole phone.
                if path.is_dir() && fully_listed(&path) {
                    let name = child.file_name().to_string_lossy().into_owned();
                    queue.push_back(if rel.is_empty() { name } else { format!("{rel}/{name}") });
                }
            }
        }
    }

    /// Refreshes a folder's placeholders from the phone (`storage.changed` or initial connect).
    #[cfg(windows)]
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

        // A full listing (it stops at MAX_LISTED otherwise) says what's no longer there.
        if entries.len() < nectarlink_core::MAX_STORAGE_LISTED {
            let on_phone: HashSet<&str> = entries.iter().map(|e| e.name.as_str()).collect();
            remove_gone(&root, rel_folder, &local_dir, &on_phone);
        }

        let mut missing: Vec<StorageEntry> = Vec::new();
        for entry in entries {
            // Windows can't show a name it can't store (`:`, `CON`, a trailing
            // dot), and `C:x` would even join to a path outside the folder.
            if nectarlink_core::storable_name(&entry.name) != entry.name {
                continue;
            }
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
            let rel = if rel_folder.is_empty() {
                entry.name.clone()
            } else {
                format!("{rel_folder}/{}", entry.name)
            };
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

    /// Removes the placeholders in `local_dir` (the phone's `rel_folder`) for
    /// what the phone no longer has. Only what's in sync goes: anything changed
    /// or added on the PC that hasn't reached the phone yet stays, and so does
    /// a folder holding some.
    #[cfg(windows)]
    fn remove_gone(root: &ActiveRoot, rel_folder: &str, local_dir: &Path, on_phone: &HashSet<&str>) {
        let Ok(children) = std::fs::read_dir(local_dir) else { return };
        for child in children.flatten() {
            let name = child.file_name().to_string_lossy().into_owned();
            if on_phone.contains(name.as_str()) || name.starts_with('.') || name.starts_with("~$") {
                continue;
            }
            let path = child.path();
            if !removable(&path) {
                continue;
            }
            let rel = if rel_folder.is_empty() { name } else { format!("{rel_folder}/{name}") };
            lock(&root.removing).insert(rel.clone());
            let removed =
                if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
            if let Err(e) = removed {
                tracing::debug!(error = %e, "can't remove a file the phone no longer has");
            }
            lock(&root.removing).remove(&rel);
            lock(&root.populated_dirs).remove(&rel);
        }
    }

    /// Whether `path` and everything under it only mirror the phone.
    #[cfg(windows)]
    fn removable(path: &Path) -> bool {
        if needs_upload(path) {
            return false;
        }
        if !path.is_dir() {
            return true;
        }
        std::fs::read_dir(path).is_ok_and(|children| children.flatten().all(|c| removable(&c.path())))
    }

    /// Watches the sync root directory for files or folders dropped in by the user
    /// and uploads them to the phone via `storage.write` / `storage.mkdir`.
    #[cfg(windows)]
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
            // Files dropped in this folder won't go to the phone: say why.
            let Some(dir_handle) = dir_handle.ok().filter(|h| *h != INVALID_HANDLE_VALUE) else {
                tracing::warn!("can't watch the phone's folder for files to upload");
                return;
            };

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

    #[cfg(windows)]
    fn parse_notify_buffer(bytes: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        let mut offset = 0usize;
        while offset + std::mem::size_of::<FILE_NOTIFY_INFORMATION>() <= bytes.len() {
            // SAFETY: offset is within bounds and 4-byte aligned in ReadDirectoryChangesW buffer.
            let info = unsafe { &*(bytes.as_ptr().add(offset) as *const FILE_NOTIFY_INFORMATION) };
            let name_bytes = info.FileNameLength as usize;
            let name_offset = offset + std::mem::offset_of!(FILE_NOTIFY_INFORMATION, FileName);
            if name_offset + name_bytes <= bytes.len()
                && matches!(
                    info.Action,
                    FILE_ACTION_ADDED | FILE_ACTION_MODIFIED | FILE_ACTION_RENAMED_NEW_NAME
                )
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
    /// Whether a placeholder folder's contents are all here (it was read
    /// from the phone), from its attributes, without listing it.
    #[cfg(windows)]
    fn fully_listed(full_path: &Path) -> bool {
        placeholder_state(full_path).is_some_and(|state| {
            (state.0 & CF_PLACEHOLDER_STATE_PLACEHOLDER.0) != 0
                && (state.0 & CF_PLACEHOLDER_STATE_PARTIAL.0) == 0
        })
    }

    #[cfg(windows)]
    fn placeholder_state(full_path: &Path) -> Option<CF_PLACEHOLDER_STATE> {
        let wide = to_wide_null(full_path);
        let mut find_data = WIN32_FIND_DATAW::default();
        // SAFETY: wide is null-terminated and find_data is a valid struct.
        let handle = unsafe { FindFirstFileW(PCWSTR(wide.as_ptr()), &mut find_data) }.ok()?;
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        // SAFETY: handle came from FindFirstFileW.
        let _ = unsafe { FindClose(handle) };
        // SAFETY: dwFileAttributes and dwReserved0 (reparse tag) were populated by FindFirstFileW.
        Some(unsafe {
            CfGetPlaceholderStateFromAttributeTag(find_data.dwFileAttributes, find_data.dwReserved0)
        })
    }

    #[cfg(windows)]
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
        let state = unsafe {
            CfGetPlaceholderStateFromAttributeTag(find_data.dwFileAttributes, find_data.dwReserved0)
        };
        if (state.0 & CF_PLACEHOLDER_STATE_PLACEHOLDER.0) == 0 {
            return true;
        }
        (state.0 & CF_PLACEHOLDER_STATE_IN_SYNC.0) == 0
    }

    #[cfg(windows)]
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

    #[cfg(windows)]
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
            let _ =
                unsafe { CfSetInSyncState(handle, CF_IN_SYNC_STATE_IN_SYNC, CF_SET_IN_SYNC_FLAG_NONE, None) };
        }
        // SAFETY: handle was opened with CreateFileW above.
        let _ = unsafe { CloseHandle(handle) };
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_failed_folder_waits_longer_each_time_up_to_half_an_hour() {
            let start = std::time::Instant::now();
            let mut retry = RetryLater::new("Pixel", "Pixel", start);
            assert!(retry.waits("Pixel", "Pixel", start));
            assert!(!retry.waits("Pixel", "Pixel", start + RetryLater::FIRST));

            let mut now = start + RetryLater::FIRST;
            retry.failed_again("Pixel", "Pixel", now);
            assert_eq!(retry.delay, RetryLater::FIRST * 2);
            assert!(retry.waits("Pixel", "Pixel", now + RetryLater::FIRST));
            for _ in 0..10 {
                now = retry.next;
                retry.failed_again("Pixel", "Pixel", now);
            }
            assert_eq!(retry.delay, RetryLater::MOST);
        }

        #[test]
        fn a_renamed_phone_is_tried_again_at_once() {
            let now = std::time::Instant::now();
            let mut retry = RetryLater::new("Pixel", "Pixel", now);
            assert!(!retry.waits("Pixel 9", "Pixel 9", now));
            retry.failed_again("Pixel 9", "Pixel 9", now);
            assert_eq!(retry.delay, RetryLater::FIRST, "a new name starts the wait over");
        }

        #[cfg(windows)]
        #[test]
        fn only_connecting_or_disconnecting_counts_as_a_link_change() {
            let phone = DeviceId([0x5a; 32]);
            assert!(online_changed(phone, true), "first time online");
            assert!(!online_changed(phone, true), "a new round-trip time isn't a change");
            assert!(online_changed(phone, false));
            assert!(!online_changed(phone, false));
            assert!(online_changed(phone, true));
        }

        #[cfg(windows)]
        #[test]
        fn clearing_a_phone_folder_keeps_files_that_never_reached_the_phone() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("Pixel");
            std::fs::create_dir_all(root.join("DCIM").join("empty")).unwrap();
            std::fs::write(root.join("DCIM").join("mine.txt"), b"not on the phone").unwrap();
            cleanup_sync_root_dir(&root);
            assert_eq!(std::fs::read(root.join("DCIM").join("mine.txt")).unwrap(), b"not on the phone");
            assert!(!root.join("DCIM").join("empty").exists(), "empty folders go");

            std::fs::remove_file(root.join("DCIM").join("mine.txt")).unwrap();
            cleanup_sync_root_dir(&root);
            assert!(!root.exists(), "nothing kept: the folder goes");
        }

        #[cfg(windows)]
        #[test]
        fn files_the_phone_never_had_are_never_removed_as_gone() {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join("Mine").join("deeper")).unwrap();
            std::fs::write(dir.path().join("Mine").join("deeper").join("notes.txt"), b"mine").unwrap();
            std::fs::write(dir.path().join("dropped.txt"), b"mine too").unwrap();
            // Plain files and folders aren't placeholders: they're waiting to go to the phone.
            assert!(!removable(&dir.path().join("dropped.txt")));
            assert!(!removable(&dir.path().join("Mine")));
        }

        #[test]
        fn converts_unix_ms_to_filetime_and_back() {
            let unix_ms = 1_710_000_000_123i64;
            let ticks = unix_ms_to_filetime_ticks(unix_ms);
            assert_eq!(ticks, EPOCH_DIFF_100NS + unix_ms * 10_000);
            assert_eq!(filetime_ticks_to_unix_ms(ticks), unix_ms);
            assert_eq!(unix_ms_to_filetime_ticks(-500), EPOCH_DIFF_100NS);
        }

        #[cfg(windows)]
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
                relative_path_under_root(
                    root,
                    Path::new(r"C:\Users\alice\Nectarlink\Pixel 9\DCIM\Camera\a.jpg")
                ),
                Some("DCIM/Camera/a.jpg".into())
            );
            assert_eq!(
                relative_path_under_root(root, Path::new(r"C:\Users\alice\Nectarlink\Pixel 9 Pro\DCIM")),
                None
            );
            // "İ" lowercases to two characters: the path still splits in the right place.
            let turkish = Path::new(r"C:\Users\alice\Nectarlink\İpek's Phone");
            assert_eq!(
                relative_path_under_root(
                    turkish,
                    Path::new(r"C:\Users\alice\Nectarlink\İpek's Phone\DCIM\a.jpg")
                ),
                Some("DCIM/a.jpg".into())
            );
            assert_eq!(
                relative_path_under_root(root, Path::new(r"c:\users\ALICE\nectarlink\pixel 9\Download")),
                Some("Download".into())
            );

            assert_eq!(sanitize_folder_name("Pixel 9 (test)"), "Pixel 9 (test)");
            assert_eq!(sanitize_folder_name("My:Phone*1?"), "My Phone 1");
            assert_eq!(sanitize_folder_name("CON"), "CON phone");
        }

        #[test]
        fn phones_with_the_same_name_get_their_own_folders() {
            let (a, b, c) = (DeviceId([1; 32]), DeviceId([2; 32]), DeviceId([3; 32]));
            let folders = folder_names([(a, "Galaxy A35"), (b, "galaxy a35"), (c, "Pixel 9")].into_iter());
            assert_eq!(folders[&a], format!("Galaxy A35 ({})", a.short()));
            assert_eq!(folders[&b], format!("galaxy a35 ({})", b.short()));
            assert_ne!(folders[&a].to_lowercase(), folders[&b].to_lowercase());
            // A phone with a name of its own keeps its plain folder.
            assert_eq!(folders[&c], "Pixel 9");
        }

        #[cfg(windows)]
        #[test]
        fn interns_callback_context_per_device_id() {
            let (a, b) = (DeviceId([7; 32]), DeviceId([8; 32]));
            let ptr_a1 = intern_device_id(a);
            let ptr_a2 = intern_device_id(a);
            let ptr_b = intern_device_id(b);
            assert_eq!(ptr_a1, ptr_a2);
            assert_ne!(ptr_a1, ptr_b);
            unsafe {
                assert_eq!(*(ptr_a1 as *const DeviceId), a);
                assert_eq!(*(ptr_b as *const DeviceId), b);
            }
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    #![allow(dead_code)]

    use nectarlink_core::{DeviceId, NodeEvent};
    use std::path::{Path, PathBuf};

    pub fn start() {}

    pub fn sync() {}

    pub fn on_event(_event: &NodeEvent) {}

    pub fn shutdown() {}

    pub fn unregister_all() {}

    pub fn sync_root_path_for(_device: DeviceId) -> Option<PathBuf> {
        None
    }

    pub fn open_in_explorer(_device: DeviceId) -> Result<(), String> {
        Err("phone storage exploration is not supported on this platform".into())
    }

    pub fn sanitize_folder_name(device_name: &str) -> String {
        device_name.chars().filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_')).collect()
    }

    pub fn unix_ms_to_filetime_ticks(unix_ms: i64) -> i64 {
        unix_ms * 10_000 + 116_444_736_000_000_000
    }

    pub fn filetime_ticks_to_unix_ms(ticks: i64) -> i64 {
        (ticks - 116_444_736_000_000_000) / 10_000
    }

    pub fn relative_path_under_root(root_path: &Path, full_path: &Path) -> Option<String> {
        full_path.strip_prefix(root_path).ok().and_then(|p| p.to_str()).map(|s| s.replace('\\', "/"))
    }
}

pub use imp::*;
