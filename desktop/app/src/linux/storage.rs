// SPDX-License-Identifier: GPL-3.0-or-later
//! Linux on-demand phone storage mount (`~/Nectarlink/<Phone name>`) via the
//! Linux kernel FUSE (`/dev/fuse` + `fusermount3`) protocol, plus GTK3
//! (`~/.config/gtk-3.0/bookmarks`) and KDE/freedesktop (`~/.local/share/user-places.xbel`)
//! file manager sidebar bookmarks.

use std::{
    collections::{HashMap, HashSet},
    fs,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::fs::{FileExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use nectarlink_core::{DeviceId, LinkState, NodeEvent, StorageEntry};

use crate::core_host;
use crate::state::{Changes, CoreStatus};
use crate::storage::{base_sync_dir, folder_names, relative_path_under_root, sanitize_folder_name};

const FUSE_KERNEL_VERSION: u32 = 7;
const FUSE_KERNEL_MINOR_VERSION: u32 = 27;
const FUSE_ROOT_ID: u64 = 1;
/// How long a mount that stopped working waits before another try.
const MOUNT_RETRY_AFTER: Duration = Duration::from_secs(30);

const FUSE_ASYNC_READ: u32 = 1 << 0;
const FUSE_ATOMIC_O_TRUNC: u32 = 1 << 3;
const FUSE_BIG_WRITES: u32 = 1 << 5;

const FATTR_SIZE: u32 = 1 << 3;

const FUSE_LOOKUP: u32 = 1;
const FUSE_FORGET: u32 = 2;
const FUSE_GETATTR: u32 = 3;
const FUSE_SETATTR: u32 = 4;
const FUSE_MKNOD: u32 = 8;
const FUSE_MKDIR: u32 = 9;
const FUSE_UNLINK: u32 = 10;
const FUSE_RMDIR: u32 = 11;
const FUSE_RENAME: u32 = 12;
const FUSE_OPEN: u32 = 14;
const FUSE_READ: u32 = 15;
const FUSE_WRITE: u32 = 16;
const FUSE_STATFS: u32 = 17;
const FUSE_RELEASE: u32 = 18;
const FUSE_FSYNC: u32 = 20;
const FUSE_FLUSH: u32 = 25;
const FUSE_INIT: u32 = 26;
const FUSE_OPENDIR: u32 = 27;
const FUSE_READDIR: u32 = 28;
const FUSE_RELEASEDIR: u32 = 29;
const FUSE_FSYNCDIR: u32 = 30;
const FUSE_ACCESS: u32 = 34;
const FUSE_CREATE: u32 = 35;
const FUSE_DESTROY: u32 = 38;
const FUSE_BATCH_FORGET: u32 = 42;
const FUSE_RENAME2: u32 = 45;

const DT_DIR: u32 = 4;
const DT_REG: u32 = 8;

const DIR_CACHE_TTL: Duration = Duration::from_secs(3);
const STREAM_READ_CHUNK_BYTES: usize = 64 * 1024;
const MAX_FUSE_MSG_LEN: usize = 256 * 1024;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseInHeader {
    pub len: u32,
    pub opcode: u32,
    pub unique: u64,
    pub nodeid: u64,
    pub uid: u32,
    pub gid: u32,
    pub pid: u32,
    pub padding: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FuseOutHeader {
    pub len: u32,
    pub error: i32,
    pub unique: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct FuseAttr {
    pub ino: u64,
    pub size: u64,
    pub blocks: u64,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub atimensec: u32,
    pub mtimensec: u32,
    pub ctimensec: u32,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u32,
    pub blksize: u32,
    pub padding: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct FuseEntryOut {
    pub nodeid: u64,
    pub generation: u64,
    pub entry_valid: u64,
    pub attr_valid: u64,
    pub entry_valid_nsec: u32,
    pub attr_valid_nsec: u32,
    pub attr: FuseAttr,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct FuseAttrOut {
    pub attr_valid: u64,
    pub attr_valid_nsec: u32,
    pub dummy: u32,
    pub attr: FuseAttr,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct FuseInitIn {
    major: u32,
    minor: u32,
    max_readahead: u32,
    flags: u32,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct FuseInitOut {
    major: u32,
    minor: u32,
    max_readahead: u32,
    flags: u32,
    max_background: u16,
    congestion_threshold: u16,
    max_write: u32,
    time_gran: u32,
    unused: [u32; 9],
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct FuseOpenIn {
    flags: u32,
    unused: u32,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Default)]
struct FuseOpenOut {
    fh: u64,
    open_flags: u32,
    padding: u32,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct FuseReadIn {
    fh: u64,
    offset: u64,
    size: u32,
    read_flags: u32,
    lock_owner: u64,
    flags: u32,
    padding: u32,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct FuseWriteIn {
    fh: u64,
    offset: u64,
    size: u32,
    write_flags: u32,
    lock_owner: u64,
    flags: u32,
    padding: u32,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct FuseWriteOut {
    size: u32,
    padding: u32,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct FuseCreateIn {
    flags: u32,
    mode: u32,
    umask: u32,
    padding: u32,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct FuseMknodIn {
    mode: u32,
    rdev: u32,
    umask: u32,
    padding: u32,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct FuseMkdirIn {
    mode: u32,
    umask: u32,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct FuseRenameIn {
    newdir: u64,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct FuseRename2In {
    newdir: u64,
    flags: u32,
    padding: u32,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct FuseSetAttrIn {
    valid: u32,
    padding: u32,
    fh: u64,
    size: u64,
    lock_owner: u64,
    atime: u64,
    mtime: u64,
    ctime: u64,
    atimensec: u32,
    mtimensec: u32,
    ctimensec: u32,
    mode: u32,
    unused4: u32,
    uid: u32,
    gid: u32,
    unused5: u32,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct FuseReleaseIn {
    fh: u64,
    flags: u32,
    release_flags: u32,
    lock_owner: u64,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct FuseFlushIn {
    fh: u64,
    unused: u32,
    padding: u32,
    lock_owner: u64,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct FuseFsyncIn {
    fh: u64,
    fsync_flags: u32,
    padding: u32,
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct FuseKStatfs {
    blocks: u64,
    bfree: u64,
    bavail: u64,
    files: u64,
    ffree: u64,
    bsize: u32,
    namelen: u32,
    frsize: u32,
    padding: u32,
    spare: [u32; 6],
}

#[repr(C)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
struct FuseDirentHeader {
    ino: u64,
    off: u64,
    namelen: u32,
    typ: u32,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn as_bytes<T: Copy>(val: &T) -> &[u8] {
    // SAFETY: `T` is a plain `#[repr(C)]` Copy struct with no uninitialized padding read hazard here.
    unsafe { std::slice::from_raw_parts((val as *const T).cast::<u8>(), std::mem::size_of::<T>()) }
}

fn read_struct<T: Copy>(bytes: &[u8]) -> Option<T> {
    if bytes.len() < std::mem::size_of::<T>() {
        return None;
    }
    // SAFETY: `bytes` has at least `size_of::<T>()` bytes and `read_unaligned` handles arbitrary alignment.
    Some(unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<T>()) })
}

fn parse_cstr(bytes: &[u8]) -> Option<&str> {
    let nul_pos = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let s = std::str::from_utf8(&bytes[..nul_pos]).ok()?;
    if s.is_empty() || s == "." || s == ".." || s.contains('/') || s.contains('\\') {
        return None;
    }
    Some(s)
}

fn parse_two_cstrs(bytes: &[u8]) -> Option<(&str, &str)> {
    let first_nul = bytes.iter().position(|&b| b == 0)?;
    let first = parse_cstr(&bytes[..first_nul])?;
    let rest = &bytes[first_nul + 1..];
    let second = parse_cstr(rest)?;
    Some((first, second))
}

/// Packs `(ino, file_type, name)` entries into 8-byte-aligned `fuse_dirent` records,
/// starting from `offset` (0-based entry index) and fitting within `max_bytes`.
pub fn pack_fuse_dirents(entries: &[(u64, u32, String)], offset: u64, max_bytes: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let header_size = std::mem::size_of::<FuseDirentHeader>();

    for (idx, (ino, typ, name)) in entries.iter().enumerate().skip(offset as usize) {
        let name_bytes = name.as_bytes();
        let rec_len = (header_size + name_bytes.len() + 7) & !7;
        if out.len() + rec_len > max_bytes {
            break;
        }
        let hdr = FuseDirentHeader {
            ino: *ino,
            off: (idx + 1) as u64,
            namelen: name_bytes.len() as u32,
            typ: *typ,
        };
        out.extend_from_slice(as_bytes(&hdr));
        out.extend_from_slice(name_bytes);
        let pad = rec_len - (header_size + name_bytes.len());
        if pad > 0 {
            out.resize(out.len() + pad, 0);
        }
    }
    out
}

/// Encodes a local directory path as a `file://` URI suitable for `~/.config/gtk-3.0/bookmarks`
/// and `~/.local/share/user-places.xbel`.
pub fn format_file_uri(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let mut encoded = String::with_capacity(raw.len() + 16);
    encoded.push_str("file://");
    for b in raw.as_bytes() {
        match *b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'/'
            | b'-'
            | b'_'
            | b'.'
            | b'~'
            | b':'
            | b'@'
            | b'('
            | b')' => encoded.push(*b as char),
            other => {
                encoded.push_str(&format!("%{other:02X}"));
            }
        }
    }
    encoded
}

/// Formats a single GTK3 bookmark entry (`file://<encoded_path> <DeviceName> (NectarLink)`).
pub fn format_gtk_bookmark_line(mount_path: &Path, device_name: &str) -> String {
    let uri = format_file_uri(mount_path);
    let label = sanitize_folder_name(device_name);
    format!("{uri} {label} (NectarLink)")
}

/// Upserts a NectarLink device bookmark line in the contents of `~/.config/gtk-3.0/bookmarks`.
pub fn upsert_gtk_bookmarks_content(existing: &str, mount_path: &Path, device_name: &str) -> String {
    let uri = format_file_uri(mount_path);
    let prefix = format!("{uri} ");
    let new_line = format_gtk_bookmark_line(mount_path, device_name);
    let mut lines: Vec<String> = Vec::new();
    let mut replaced = false;
    for line in existing.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == uri || trimmed.starts_with(&prefix) {
            if !replaced {
                lines.push(new_line.clone());
                replaced = true;
            }
        } else {
            lines.push(trimmed.to_owned());
        }
    }
    if !replaced {
        lines.push(new_line);
    }
    if lines.is_empty() { String::new() } else { format!("{}\n", lines.join("\n")) }
}

/// Removes a NectarLink device bookmark line matching `mount_path` from `~/.config/gtk-3.0/bookmarks` content.
pub fn remove_gtk_bookmarks_content(existing: &str, mount_path: &Path) -> String {
    let uri = format_file_uri(mount_path);
    let prefix = format!("{uri} ");
    let lines: Vec<&str> = existing
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty() && *line != uri && !line.starts_with(&prefix))
        .collect();
    if lines.is_empty() { String::new() } else { format!("{}\n", lines.join("\n")) }
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// Upserts a NectarLink device bookmark in KDE Dolphin / freedesktop `~/.local/share/user-places.xbel`.
pub fn upsert_xbel_bookmarks_content(existing: &str, mount_path: &Path, device_name: &str) -> String {
    let cleaned = remove_xbel_bookmarks_content(existing, mount_path);
    let uri = escape_xml(&format_file_uri(mount_path));
    let label = escape_xml(&format!("{} (NectarLink)", sanitize_folder_name(device_name)));
    let entry = format!(
        " <bookmark href=\"{uri}\" id=\"nectarlink\"><title>{label}</title><info><metadata owner=\"http://freedesktop.org\"><bookmark:icon name=\"smartphone\"/></metadata></info></bookmark>"
    );

    if let Some(close_idx) = cleaned.rfind("</xbel>") {
        let mut out = String::with_capacity(cleaned.len() + entry.len() + 2);
        out.push_str(&cleaned[..close_idx]);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&entry);
        out.push('\n');
        out.push_str(&cleaned[close_idx..]);
        out
    } else {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE xbel>\n<xbel xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\" xmlns:kdepriv=\"http://www.kde.org/kdepriv\" xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\">\n{entry}\n</xbel>\n"
        )
    }
}

/// Removes a NectarLink device bookmark matching `mount_path` from `~/.local/share/user-places.xbel` content.
pub fn remove_xbel_bookmarks_content(existing: &str, mount_path: &Path) -> String {
    if existing.trim().is_empty() {
        return String::new();
    }
    let uri = escape_xml(&format_file_uri(mount_path));
    let needle = format!("href=\"{uri}\"");
    remove_xbel_blocks_matching(existing, |block| block.contains(&needle))
}

fn remove_xbel_blocks_matching(existing: &str, predicate: impl Fn(&str) -> bool) -> String {
    let mut out = String::with_capacity(existing.len());
    let mut rest = existing;
    while let Some(start) = rest.find("<bookmark ") {
        let prefix = &rest[..start];
        let after_start = &rest[start..];
        let Some(end_rel) = after_start.find("</bookmark>") else {
            out.push_str(rest);
            return out;
        };
        let end = end_rel + "</bookmark>".len();
        let block = &after_start[..end];
        if predicate(block) {
            let trimmed_prefix = prefix.strip_suffix(' ').unwrap_or(prefix);
            out.push_str(trimmed_prefix);
            let mut next = &after_start[end..];
            if let Some(stripped) = next.strip_prefix("\r\n") {
                next = stripped;
            } else if let Some(stripped) = next.strip_prefix('\n') {
                next = stripped;
            }
            rest = next;
        } else {
            out.push_str(prefix);
            out.push_str(block);
            rest = &after_start[end..];
        }
    }
    out.push_str(rest);
    out
}

/// Converts a local path inside `mount_root` into a normalized forward-slash relative remote path
/// (`""` for `mount_root` itself). Returns `None` if `local_path` is outside `mount_root` or
/// contains parent/current directory traversal components.
pub fn to_remote_rel_path(mount_root: &Path, local_path: &Path) -> Option<String> {
    if let Ok(stripped) = local_path.strip_prefix(mount_root) {
        let mut parts: Vec<String> = Vec::new();
        for comp in stripped.components() {
            match comp {
                Component::Normal(os) => {
                    let s = os.to_string_lossy();
                    if !s.is_empty() {
                        parts.push(s.into_owned());
                    }
                }
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
            }
        }
        return Some(parts.join("/"));
    }
    relative_path_under_root(mount_root, local_path)
}

#[derive(Debug, Clone)]
struct InodeMeta {
    is_dir: bool,
    size: u64,
    modified_ms: i64,
}

/// A file being written through the mount, kept on disk until it's uploaded:
/// a phone holds videos larger than this PC's memory.
#[derive(Debug)]
struct Spool {
    file: fs::File,
    path: PathBuf,
    len: u64,
}

impl Spool {
    fn create(dir: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(dir)?;
        let seq = STAGE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("upload-{}-{seq}.tmp", std::process::id()));
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        Ok(Self { file, path, len: 0 })
    }

    fn write_at(&mut self, offset: u64, bytes: &[u8]) -> std::io::Result<()> {
        self.file.write_all_at(bytes, offset)?;
        self.len = self.len.max(offset.saturating_add(bytes.len() as u64));
        Ok(())
    }

    /// Up to `want` bytes at `offset`; fewer at the end of the file.
    fn read_at(&self, offset: u64, want: usize) -> std::io::Result<Vec<u8>> {
        let want = (want as u64).min(self.len.saturating_sub(offset)) as usize;
        let mut out = vec![0; want];
        self.file.read_exact_at(&mut out, offset)?;
        Ok(out)
    }

    fn set_len(&mut self, len: u64) -> std::io::Result<()> {
        self.file.set_len(len)?;
        self.len = len;
        Ok(())
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Debug)]
struct OpenFileHandle {
    nodeid: u64,
    rel_path: String,
    dirty: bool,
    /// The file's contents once it has been truncated or written to.
    spool: Option<Spool>,
    read_cache_offset: u64,
    read_cache_data: Vec<u8>,
}

struct FuseState {
    device: DeviceId,
    uid: u32,
    gid: u32,
    next_ino: u64,
    next_fh: u64,
    ino_to_path: HashMap<u64, String>,
    path_to_ino: HashMap<String, u64>,
    meta_by_ino: HashMap<u64, InodeMeta>,
    dir_cache: HashMap<String, (Instant, Vec<StorageEntry>)>,
    handles: HashMap<u64, OpenFileHandle>,
}

impl FuseState {
    fn new(device: DeviceId) -> Self {
        // SAFETY: `geteuid` and `getegid` are always safe.
        let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
        let mut ino_to_path = HashMap::new();
        let mut path_to_ino = HashMap::new();
        let mut meta_by_ino = HashMap::new();
        ino_to_path.insert(FUSE_ROOT_ID, String::new());
        path_to_ino.insert(String::new(), FUSE_ROOT_ID);
        meta_by_ino.insert(FUSE_ROOT_ID, InodeMeta { is_dir: true, size: 4096, modified_ms: 0 });
        Self {
            device,
            uid,
            gid,
            next_ino: 2,
            next_fh: 1,
            ino_to_path,
            path_to_ino,
            meta_by_ino,
            dir_cache: HashMap::new(),
            handles: HashMap::new(),
        }
    }

    fn invalidate_all(&mut self) {
        self.dir_cache.clear();
    }

    fn invalidate_dir(&mut self, dir_rel: &str) {
        self.dir_cache.remove(dir_rel);
    }

    fn intern_path(&mut self, rel_path: &str, meta: InodeMeta) -> u64 {
        let ino = if let Some(&existing) = self.path_to_ino.get(rel_path) {
            existing
        } else {
            let id = self.next_ino;
            self.next_ino += 1;
            self.path_to_ino.insert(rel_path.to_owned(), id);
            self.ino_to_path.insert(id, rel_path.to_owned());
            id
        };
        self.meta_by_ino.insert(ino, meta);
        ino
    }

    /// Moves a renamed file or folder, and everything under it, to its new path,
    /// so open files and paths the kernel still holds keep working.
    fn rename_path(&mut self, old_rel: &str, new_rel: &str) {
        if let Some(replaced) = self.path_to_ino.remove(new_rel) {
            self.ino_to_path.remove(&replaced);
            self.meta_by_ino.remove(&replaced);
        }
        let moved = |path: &str| {
            let rest = path.strip_prefix(old_rel)?;
            (rest.is_empty() || rest.starts_with('/')).then(|| format!("{new_rel}{rest}"))
        };
        let renames: Vec<(String, u64, String)> = self
            .path_to_ino
            .iter()
            .filter_map(|(path, ino)| moved(path).map(|new_path| (path.clone(), *ino, new_path)))
            .collect();
        for (path, ino, new_path) in renames {
            self.path_to_ino.remove(&path);
            self.ino_to_path.insert(ino, new_path.clone());
            self.path_to_ino.insert(new_path, ino);
        }
        for handle in self.handles.values_mut() {
            if let Some(new_path) = moved(&handle.rel_path) {
                handle.rel_path = new_path;
            }
        }
    }

    /// The size of a file with unsaved writes, which the phone doesn't know yet.
    fn unsaved_len(&self, ino: u64) -> Option<u64> {
        self.handles
            .values()
            .filter(|h| h.nodeid == ino && h.dirty)
            .find_map(|h| h.spool.as_ref().map(|spool| spool.len))
    }

    fn build_attr(&self, ino: u64, meta: &InodeMeta) -> FuseAttr {
        let (sec, nsec) = if meta.modified_ms > 0 {
            let ms = meta.modified_ms as u64;
            (ms / 1000, ((ms % 1000) * 1_000_000) as u32)
        } else {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
            (now.as_secs(), now.subsec_nanos())
        };
        let (mode, nlink, size) = if meta.is_dir {
            (libc::S_IFDIR | 0o755, 2, 4096)
        } else {
            (libc::S_IFREG | 0o644, 1, meta.size)
        };
        FuseAttr {
            ino,
            size,
            blocks: size.div_ceil(512),
            atime: sec,
            mtime: sec,
            ctime: sec,
            atimensec: nsec,
            mtimensec: nsec,
            ctimensec: nsec,
            mode,
            nlink,
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: 4096,
            padding: 0,
        }
    }

    fn entry_out(&self, ino: u64, meta: &InodeMeta) -> FuseEntryOut {
        FuseEntryOut {
            nodeid: ino,
            generation: 1,
            entry_valid: 2,
            attr_valid: 2,
            entry_valid_nsec: 0,
            attr_valid_nsec: 0,
            attr: self.build_attr(ino, meta),
        }
    }

    fn attr_out(&self, ino: u64, meta: &InodeMeta) -> FuseAttrOut {
        FuseAttrOut { attr_valid: 2, attr_valid_nsec: 0, dummy: 0, attr: self.build_attr(ino, meta) }
    }
}

fn join_rel_path(parent: &str, name: &str) -> String {
    if parent.is_empty() { name.to_owned() } else { format!("{parent}/{name}") }
}

fn parent_and_name(rel_path: &str) -> (&str, &str) {
    match rel_path.rsplit_once('/') {
        Some((p, n)) => (p, n),
        None => ("", rel_path),
    }
}

struct MountedDevice {
    device: DeviceId,
    device_name: String,
    mount_path: PathBuf,
    stop: Arc<AtomicBool>,
    fuse_state: Arc<Mutex<FuseState>>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// When the mount stopped working on its own: it failed to start, or
    /// something else unmounted it.
    died_at: Mutex<Option<Instant>>,
}

impl MountedDevice {
    fn retry_due(&self) -> bool {
        lock(&self.died_at).is_some_and(|at| at.elapsed() >= MOUNT_RETRY_AFTER)
    }

    fn is_dead(&self) -> bool {
        lock(&self.died_at).is_some()
    }

    fn invalidate_cache(&self) {
        lock(&self.fuse_state).invalidate_all();
    }

    fn stop_and_join(&self) {
        self.stop.store(true, Ordering::Release);
        unmount_fuse_path(&self.mount_path);
        if let Some(handle) = lock(&self.worker).take() {
            let _ = handle.join();
        }
        unmount_fuse_path(&self.mount_path);
    }
}

static MOUNTS: OnceLock<Mutex<HashMap<DeviceId, Arc<MountedDevice>>>> = OnceLock::new();
static LINUX_RECONCILING: Mutex<()> = Mutex::new(());
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
/// Whether each phone was connected when its link last changed. The link
/// also changes with every round-trip time update, which isn't worth a sync.
static LINK_ONLINE: Mutex<Option<HashMap<DeviceId, bool>>> = Mutex::new(None);

/// Whether a link change moved `device` between connected and not, noting where it is now.
fn online_changed(device: DeviceId, online: bool) -> bool {
    lock(&LINK_ONLINE).get_or_insert_with(HashMap::new).insert(device, online) != Some(online)
}
static STAGE_COUNTER: AtomicU64 = AtomicU64::new(1);

fn mounts() -> MutexGuard<'static, HashMap<DeviceId, Arc<MountedDevice>>> {
    let map = MOUNTS.get_or_init(|| Mutex::new(HashMap::new()));
    lock(map)
}

fn get_mounted_device(device: &DeviceId) -> Option<Arc<MountedDevice>> {
    mounts().get(device).cloned()
}

pub fn mounted_path_for(device: &DeviceId) -> Option<PathBuf> {
    get_mounted_device(device).filter(|m| !m.is_dead()).map(|m| m.mount_path.clone())
}

pub fn sync_root_path_for(device: &DeviceId) -> Option<PathBuf> {
    mounted_path_for(device)
}

pub fn open_in_explorer(device: &DeviceId) -> bool {
    let Some(path) = mounted_path_for(device) else {
        return false;
    };
    super::shell::open_folder(&path).is_ok()
}

fn gtk_bookmarks_path() -> Option<PathBuf> {
    dirs::config_dir()
        .filter(|p| p.is_absolute())
        .or_else(|| dirs::home_dir().filter(|p| p.is_absolute()).map(|h| h.join(".config")))
        .map(|cfg| cfg.join("gtk-3.0").join("bookmarks"))
}

fn xbel_bookmarks_path() -> Option<PathBuf> {
    dirs::data_local_dir()
        .filter(|p| p.is_absolute())
        .or_else(|| dirs::home_dir().filter(|p| p.is_absolute()).map(|h| h.join(".local").join("share")))
        .map(|dir| dir.join("user-places.xbel"))
}

fn register_sidebar_bookmarks(mount_path: &Path, device_name: &str) {
    if let Some(bookmarks_file) = gtk_bookmarks_path() {
        if let Some(parent) = bookmarks_file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let existing = fs::read_to_string(&bookmarks_file).unwrap_or_default();
        let updated = upsert_gtk_bookmarks_content(&existing, mount_path, device_name);
        let _ = fs::write(&bookmarks_file, updated);
    }
    if let Some(xbel_file) = xbel_bookmarks_path() {
        if let Some(parent) = xbel_file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let existing = fs::read_to_string(&xbel_file).unwrap_or_default();
        let updated = upsert_xbel_bookmarks_content(&existing, mount_path, device_name);
        let _ = fs::write(&xbel_file, updated);
    }
}

fn unregister_sidebar_bookmarks(mount_path: &Path) {
    if let Some(bookmarks_file) = gtk_bookmarks_path()
        && let Ok(existing) = fs::read_to_string(&bookmarks_file)
    {
        let updated = remove_gtk_bookmarks_content(&existing, mount_path);
        let _ = fs::write(&bookmarks_file, updated);
    }
    if let Some(xbel_file) = xbel_bookmarks_path()
        && let Ok(existing) = fs::read_to_string(&xbel_file)
    {
        let updated = remove_xbel_bookmarks_content(&existing, mount_path);
        let _ = fs::write(&xbel_file, updated);
    }
}

fn find_fusermount_binary() -> Option<&'static str> {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let extra = [Path::new("/usr/bin"), Path::new("/bin"), Path::new("/usr/sbin"), Path::new("/sbin")];
    for candidate_name in ["fusermount3", "fusermount"] {
        let found =
            std::env::split_paths(&path_var).chain(extra.into_iter().map(Path::to_path_buf)).any(|dir| {
                let p = dir.join(candidate_name);
                fs::metadata(p).is_ok_and(|m| m.is_file() && (m.mode() & 0o111 != 0))
            });
        if found {
            return Some(candidate_name);
        }
    }
    None
}

/// A mount left by a copy of the app that ended without unmounting (a crash,
/// `kill -9`) answers every access with "transport endpoint is not connected"
/// and blocks a new mount at the same place: detach it first.
fn clear_stale_mount(mount_path: &Path) {
    let disconnected = fs::metadata(mount_path).err().and_then(|e| e.raw_os_error()) == Some(libc::ENOTCONN);
    let listed =
        fs::read_to_string("/proc/self/mounts").is_ok_and(|mounts| is_listed_mount(&mounts, mount_path));
    if disconnected || listed {
        tracing::info!("detaching a phone storage mount left by an earlier run");
        unmount_fuse_path(mount_path);
    }
}

/// Whether `/proc/self/mounts` text lists a NectarLink FUSE mount at `path`
/// (mount points there escape spaces, tabs, newlines and backslashes as octal).
pub(crate) fn is_listed_mount(mounts: &str, path: &Path) -> bool {
    fn unescape(field: &str) -> String {
        let mut out = String::with_capacity(field.len());
        let mut chars = field.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                let code: String = chars.by_ref().take(3).collect();
                match u8::from_str_radix(&code, 8) {
                    Ok(byte) => out.push(char::from(byte)),
                    Err(_) => {
                        out.push('\\');
                        out.push_str(&code);
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }
    let wanted = path.to_string_lossy();
    mounts.lines().any(|line| {
        let mut fields = line.split(' ');
        let (_source, Some(point), Some(kind)) = (fields.next(), fields.next(), fields.next()) else {
            return false;
        };
        kind == "fuse.nectarlink" && unescape(point) == wanted
    })
}

fn unmount_fuse_path(mount_path: &Path) {
    for bin in ["fusermount3", "fusermount"] {
        if let Ok(status) = Command::new(bin)
            .args(["-u", "-z", "--"])
            .arg(mount_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            && status.success()
        {
            return;
        }
    }
}

#[repr(C)]
#[allow(dead_code)]
union CmsgAligned {
    cmsg: libc::cmsghdr,
    buf: [u8; 64],
}

fn recv_fd_via_scm_rights(sock_fd: i32) -> Option<OwnedFd> {
    let tv = libc::timeval { tv_sec: 5, tv_usec: 0 };
    // SAFETY: setting `SO_RCVTIMEO` on a valid socket fd.
    unsafe {
        let _ = libc::setsockopt(
            sock_fd,
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            (&raw const tv).cast(),
            std::mem::size_of::<libc::timeval>() as u32,
        );
    }

    let mut data_buf = [0u8; 1];
    let mut iov = libc::iovec { iov_base: data_buf.as_mut_ptr().cast(), iov_len: data_buf.len() };
    let mut cmsg_storage = CmsgAligned { buf: [0u8; 64] };
    // SAFETY: `CMSG_SPACE` is a pure size calculation macro.
    let cmsg_space = unsafe { libc::CMSG_SPACE(std::mem::size_of::<i32>() as u32) as usize };

    // SAFETY: zero-initializing `msghdr` and setting pointers to live stack buffers.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    // SAFETY: accessing the union byte buffer for `msg_control`.
    msg.msg_control = unsafe { cmsg_storage.buf.as_mut_ptr().cast() };
    msg.msg_controllen = cmsg_space as _;

    // SAFETY: `msg` references live stack buffers and `sock_fd` is open.
    let n = unsafe { libc::recvmsg(sock_fd, &mut msg, 0) };
    if n < 0 {
        return None;
    }

    // SAFETY: walking the kernel-populated control message headers.
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(&msg);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let fd_ptr = libc::CMSG_DATA(cmsg).cast::<i32>();
                let received_fd = std::ptr::read_unaligned(fd_ptr);
                if received_fd >= 0 {
                    let _ = libc::fcntl(received_fd, libc::F_SETFD, libc::FD_CLOEXEC);
                    return Some(OwnedFd::from_raw_fd(received_fd));
                }
            }
            cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
        }
    }
    None
}

fn open_fuse_mount(mount_path: &Path) -> Option<OwnedFd> {
    let fusermount_bin = find_fusermount_binary()?;
    unmount_fuse_path(mount_path);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::create_dir_all(mount_path);
        let _ = fs::set_permissions(mount_path, fs::Permissions::from_mode(0o700));
    }

    let mut sv = [0i32; 2];
    // SAFETY: creating a connected Unix stream socket pair.
    if unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0, sv.as_mut_ptr()) }
        != 0
    {
        return None;
    }
    let parent_sock = sv[0];
    let child_sock = sv[1];

    // Clear FD_CLOEXEC on `child_sock` so `fusermount3` inherits `_FUSE_COMMFD`.
    // SAFETY: `child_sock` is a valid open descriptor from `socketpair`.
    unsafe {
        let _ = libc::fcntl(child_sock, libc::F_SETFD, 0);
    }

    let child_res = Command::new(fusermount_bin)
        .env("_FUSE_COMMFD", child_sock.to_string())
        .args(["-o", "rw,nosuid,nodev,default_permissions,fsname=nectarlink,subtype=nectarlink", "--"])
        .arg(mount_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    // SAFETY: parent no longer needs `child_sock` after spawning.
    unsafe {
        let _ = libc::close(child_sock);
    }

    let mut child = match child_res {
        Ok(c) => c,
        Err(_) => {
            // SAFETY: closing `parent_sock` on spawn error.
            unsafe {
                let _ = libc::close(parent_sock);
            }
            return None;
        }
    };

    let fuse_fd = recv_fd_via_scm_rights(parent_sock);
    // SAFETY: closing `parent_sock` after receiving the `/dev/fuse` fd.
    unsafe {
        let _ = libc::close(parent_sock);
    }
    let _ = child.wait();
    fuse_fd
}

fn send_fuse_reply(fuse_fd: i32, unique: u64, error: i32, payload: &[u8]) {
    let hdr =
        FuseOutHeader { len: (std::mem::size_of::<FuseOutHeader>() + payload.len()) as u32, error, unique };
    let hdr_bytes = as_bytes(&hdr);
    let mut iov = [
        libc::iovec { iov_base: hdr_bytes.as_ptr().cast_mut().cast(), iov_len: hdr_bytes.len() },
        libc::iovec { iov_base: payload.as_ptr().cast_mut().cast(), iov_len: payload.len() },
    ];
    let iovcnt = if payload.is_empty() { 1 } else { 2 };
    // SAFETY: `iov` points to live slices (`hdr_bytes` and `payload`) for the duration of `writev`.
    unsafe {
        let _ = libc::writev(fuse_fd, iov.as_mut_ptr(), iovcnt);
    }
}

fn fetch_dir_entries(state: &mut FuseState, dir_rel: &str) -> Result<Vec<StorageEntry>, i32> {
    if let Some((fetched_at, entries)) = state.dir_cache.get(dir_rel)
        && fetched_at.elapsed() < DIR_CACHE_TTL
    {
        return Ok(entries.clone());
    }
    let Some(node) = core_host::node() else {
        return Err(-libc::ENOTCONN);
    };
    let device = state.device;
    let rel_owned = dir_rel.to_owned();
    let entries = core_host::host()
        .runtime
        .block_on(async move { node.storage_list(device, &rel_owned).await })
        .map_err(|e| storage_errno(&e))?;
    for entry in &entries {
        let name = entry.name.trim_matches('/');
        if !name.is_empty() && name != "." && name != ".." && !name.contains('/') {
            let child_rel = join_rel_path(dir_rel, name);
            state.intern_path(
                &child_rel,
                InodeMeta { is_dir: entry.is_dir, size: entry.size, modified_ms: entry.modified },
            );
        }
    }
    state.dir_cache.insert(dir_rel.to_owned(), (Instant::now(), entries.clone()));
    Ok(entries)
}

fn refresh_inode_meta(state: &mut FuseState, ino: u64) -> Result<InodeMeta, i32> {
    if ino == FUSE_ROOT_ID {
        return Ok(InodeMeta { is_dir: true, size: 4096, modified_ms: 0 });
    }
    let Some(rel_path) = state.ino_to_path.get(&ino).cloned() else {
        return Err(-libc::ENOENT);
    };
    let (parent_rel, file_name) = parent_and_name(&rel_path);
    if let Ok(entries) = fetch_dir_entries(state, parent_rel)
        && let Some(found) = entries.iter().find(|e| e.name.trim_matches('/') == file_name)
    {
        let size = state.unsaved_len(ino).unwrap_or(found.size);
        let meta = InodeMeta { is_dir: found.is_dir, size, modified_ms: found.modified };
        state.meta_by_ino.insert(ino, meta.clone());
        return Ok(meta);
    }
    state.meta_by_ino.get(&ino).cloned().ok_or(-libc::ENOENT)
}

fn read_remote_slice(device: DeviceId, rel_path: &str, offset: u64, size: usize) -> Result<Vec<u8>, i32> {
    if size == 0 {
        return Ok(Vec::new());
    }
    let Some(node) = core_host::node() else {
        return Err(-libc::ENOTCONN);
    };
    let rel_owned = rel_path.to_owned();
    let max_len = size as u64;
    core_host::host().runtime.block_on(async move {
        let (_, mut recv) = node
            .storage_read_stream(device, &rel_owned, offset, Some(max_len))
            .await
            .map_err(|e| storage_errno(&e))?;
        let mut out = Vec::with_capacity(size.min(128 * 1024));
        while out.len() < size {
            let want = (size - out.len()).min(STREAM_READ_CHUNK_BYTES);
            match recv.read_chunk(want).await {
                Ok(Some(chunk)) => {
                    if chunk.is_empty() {
                        break;
                    }
                    out.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(_) => return Err(-libc::EIO),
            }
        }
        if out.len() > size {
            out.truncate(size);
        }
        Ok(out)
    })
}

fn io_errno(error: &std::io::Error) -> i32 {
    -error.raw_os_error().unwrap_or(libc::EIO)
}

/// The errno a failed phone request reports to the file manager, so "the
/// phone is offline" and "the phone refused" don't both read as an I/O error.
/// Logs only the kind of failure, never a path or file name.
fn storage_errno(error: &nectarlink_core::Error) -> i32 {
    use nectarlink_core::Error;
    let (errno, kind) = match error {
        Error::Offline | Error::NotPaired => (libc::ENOTCONN, "offline"),
        Error::Denied | Error::Declined => (libc::EACCES, "denied"),
        Error::NotFound => (libc::ENOENT, "not found"),
        Error::Timeout => (libc::ETIMEDOUT, "timed out"),
        Error::Unsupported => (libc::EOPNOTSUPP, "unsupported"),
        Error::TooLarge => (libc::EFBIG, "too large"),
        Error::Io(e) => (e.raw_os_error().unwrap_or(libc::EIO), "i/o"),
        _ => (libc::EIO, "other"),
    };
    tracing::debug!(errno, kind, "phone storage request failed");
    -errno
}

fn new_spool() -> Result<Spool, i32> {
    Spool::create(&core_host::host().data_dir.join("fuse-stage")).map_err(|e| io_errno(&e))
}

fn upload_file(device: DeviceId, rel_path: &str, source: &Path) -> Result<(), i32> {
    let Some(node) = core_host::node() else {
        return Err(-libc::ENOTCONN);
    };
    let rel_owned = rel_path.to_owned();
    let source = source.to_owned();
    let res = core_host::host()
        .runtime
        .block_on(async move { node.storage_write(device, &rel_owned, &source).await });
    res.map(|_| ()).map_err(|e| storage_errno(&e))
}

/// The phone's copy of a file, on disk. A failed read is an error rather than an
/// empty file, which an upload would then save over the real one.
fn download_to_spool(device: DeviceId, rel_path: &str) -> Result<Spool, i32> {
    let Some(node) = core_host::node() else {
        return Err(-libc::ENOTCONN);
    };
    let mut spool = new_spool()?;
    let rel_owned = rel_path.to_owned();
    core_host::host().runtime.block_on(async {
        let (_, mut recv) =
            node.storage_read_stream(device, &rel_owned, 0, None).await.map_err(|e| storage_errno(&e))?;
        loop {
            match recv.read_chunk(STREAM_READ_CHUNK_BYTES).await {
                Ok(Some(chunk)) if chunk.is_empty() => return Ok(()),
                Ok(Some(chunk)) => spool.write_at(spool.len, &chunk).map_err(|e| io_errno(&e))?,
                Ok(None) => return Ok(()),
                Err(_) => return Err(-libc::EIO),
            }
        }
    })?;
    Ok(spool)
}

/// Gives an open file its spool, starting from the phone's copy, before the first
/// change that keeps what's already there.
fn load_for_writing(state_arc: &Arc<Mutex<FuseState>>, fh: u64) -> Result<(), i32> {
    let (device, rel_path, size) = {
        let state = lock(state_arc);
        let Some(h) = state.handles.get(&fh) else {
            return Err(-libc::EBADF);
        };
        if h.spool.is_some() {
            return Ok(());
        }
        let size = state.meta_by_ino.get(&h.nodeid).map_or(0, |m| m.size);
        (state.device, h.rel_path.clone(), size)
    };
    let spool = if size > 0 { download_to_spool(device, &rel_path)? } else { new_spool()? };
    let mut state = lock(state_arc);
    let Some(h) = state.handles.get_mut(&fh) else {
        return Err(-libc::EBADF);
    };
    h.spool = Some(spool);
    Ok(())
}

/// Resizes `ino`'s open handles or, with none open, the phone's file itself.
fn truncate_file(state_arc: &Arc<Mutex<FuseState>>, ino: u64, rel_path: &str, size: u64) -> Result<(), i32> {
    let (device, handles) = {
        let state = lock(state_arc);
        let handles: Vec<u64> =
            state.handles.iter().filter(|(_, h)| h.nodeid == ino).map(|(fh, _)| *fh).collect();
        (state.device, handles)
    };
    if handles.is_empty() {
        let mut spool = if size == 0 { new_spool()? } else { download_to_spool(device, rel_path)? };
        spool.set_len(size).map_err(|e| io_errno(&e))?;
        upload_file(device, rel_path, &spool.path)?;
        let (parent_rel, _) = parent_and_name(rel_path);
        lock(state_arc).invalidate_dir(parent_rel);
        return Ok(());
    }
    for fh in handles {
        if size == 0 {
            let spool = new_spool()?;
            if let Some(h) = lock(state_arc).handles.get_mut(&fh) {
                h.spool = Some(spool);
            }
        } else {
            load_for_writing(state_arc, fh)?;
        }
        if let Some(h) = lock(state_arc).handles.get_mut(&fh)
            && let Some(spool) = h.spool.as_mut()
        {
            spool.set_len(size).map_err(|e| io_errno(&e))?;
            h.dirty = true;
        }
    }
    Ok(())
}

fn handle_fuse_request(
    fuse_fd: i32,
    state_arc: &Arc<Mutex<FuseState>>,
    hdr: FuseInHeader,
    body: &[u8],
) -> bool {
    match hdr.opcode {
        FUSE_INIT => {
            let Some(init_in) = read_struct::<FuseInitIn>(body) else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EINVAL, &[]);
                return true;
            };
            let reply = FuseInitOut {
                major: FUSE_KERNEL_VERSION,
                minor: init_in.minor.min(FUSE_KERNEL_MINOR_VERSION),
                max_readahead: init_in.max_readahead.max(128 * 1024),
                flags: (init_in.flags & (FUSE_ASYNC_READ | FUSE_BIG_WRITES | FUSE_ATOMIC_O_TRUNC)),
                max_background: 16,
                congestion_threshold: 12,
                max_write: 128 * 1024,
                time_gran: 1_000_000_000,
                unused: [0; 9],
            };
            send_fuse_reply(fuse_fd, hdr.unique, 0, as_bytes(&reply));
        }
        FUSE_DESTROY => {
            send_fuse_reply(fuse_fd, hdr.unique, 0, &[]);
            return false;
        }
        FUSE_FORGET | FUSE_BATCH_FORGET => {
            // No reply for FORGET notifications.
        }
        FUSE_ACCESS | FUSE_FSYNCDIR | FUSE_RELEASEDIR => {
            send_fuse_reply(fuse_fd, hdr.unique, 0, &[]);
        }
        FUSE_STATFS => {
            let kstat = FuseKStatfs {
                blocks: 16 * 1024 * 1024,
                bfree: 8 * 1024 * 1024,
                bavail: 8 * 1024 * 1024,
                files: 1_000_000,
                ffree: 900_000,
                bsize: 4096,
                namelen: 255,
                frsize: 4096,
                padding: 0,
                spare: [0; 6],
            };
            send_fuse_reply(fuse_fd, hdr.unique, 0, as_bytes(&kstat));
        }
        FUSE_LOOKUP => {
            let Some(child_name) = parse_cstr(body) else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::ENOENT, &[]);
                return true;
            };
            let mut state = lock(state_arc);
            let Some(parent_rel) = state.ino_to_path.get(&hdr.nodeid).cloned() else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::ENOENT, &[]);
                return true;
            };
            let entries = match fetch_dir_entries(&mut state, &parent_rel) {
                Ok(e) => e,
                Err(err) => {
                    send_fuse_reply(fuse_fd, hdr.unique, err, &[]);
                    return true;
                }
            };
            let child_rel = join_rel_path(&parent_rel, child_name);
            if let Some(found) = entries.iter().find(|e| e.name.trim_matches('/') == child_name) {
                let mut meta =
                    InodeMeta { is_dir: found.is_dir, size: found.size, modified_ms: found.modified };
                if let Some(&ino) = state.path_to_ino.get(&child_rel)
                    && let Some(len) = state.unsaved_len(ino)
                {
                    meta.size = len;
                }
                let ino = state.intern_path(&child_rel, meta.clone());
                let out = state.entry_out(ino, &meta);
                send_fuse_reply(fuse_fd, hdr.unique, 0, as_bytes(&out));
            } else if let Some(&ino) = state.path_to_ino.get(&child_rel)
                && state.handles.values().any(|h| h.nodeid == ino)
                && let Some(meta) = state.meta_by_ino.get(&ino).cloned()
            {
                let out = state.entry_out(ino, &meta);
                send_fuse_reply(fuse_fd, hdr.unique, 0, as_bytes(&out));
            } else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::ENOENT, &[]);
            }
        }
        FUSE_GETATTR => {
            let mut state = lock(state_arc);
            match refresh_inode_meta(&mut state, hdr.nodeid) {
                Ok(meta) => {
                    let out = state.attr_out(hdr.nodeid, &meta);
                    send_fuse_reply(fuse_fd, hdr.unique, 0, as_bytes(&out));
                }
                Err(err) => send_fuse_reply(fuse_fd, hdr.unique, err, &[]),
            }
        }
        FUSE_SETATTR => {
            let Some(setattr_in) = read_struct::<FuseSetAttrIn>(body) else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EINVAL, &[]);
                return true;
            };
            let Some(rel_path) = lock(state_arc).ino_to_path.get(&hdr.nodeid).cloned() else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::ENOENT, &[]);
                return true;
            };
            if (setattr_in.valid & FATTR_SIZE) != 0 {
                if let Err(err) = truncate_file(state_arc, hdr.nodeid, &rel_path, setattr_in.size) {
                    send_fuse_reply(fuse_fd, hdr.unique, err, &[]);
                    return true;
                }
                if let Some(meta) = lock(state_arc).meta_by_ino.get_mut(&hdr.nodeid) {
                    meta.size = setattr_in.size;
                }
            }
            let state = lock(state_arc);
            let meta = state.meta_by_ino.get(&hdr.nodeid).cloned().unwrap_or(InodeMeta {
                is_dir: false,
                size: 0,
                modified_ms: 0,
            });
            let out = state.attr_out(hdr.nodeid, &meta);
            send_fuse_reply(fuse_fd, hdr.unique, 0, as_bytes(&out));
        }
        FUSE_OPENDIR => {
            let mut state = lock(state_arc);
            if !state.ino_to_path.contains_key(&hdr.nodeid) {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::ENOENT, &[]);
                return true;
            }
            let fh = state.next_fh;
            state.next_fh += 1;
            let out = FuseOpenOut { fh, open_flags: 0, padding: 0 };
            send_fuse_reply(fuse_fd, hdr.unique, 0, as_bytes(&out));
        }
        FUSE_READDIR => {
            let Some(read_in) = read_struct::<FuseReadIn>(body) else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EINVAL, &[]);
                return true;
            };
            let mut state = lock(state_arc);
            let Some(dir_rel) = state.ino_to_path.get(&hdr.nodeid).cloned() else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::ENOENT, &[]);
                return true;
            };
            let entries = match fetch_dir_entries(&mut state, &dir_rel) {
                Ok(e) => e,
                Err(err) => {
                    send_fuse_reply(fuse_fd, hdr.unique, err, &[]);
                    return true;
                }
            };
            let mut dirents: Vec<(u64, u32, String)> = Vec::with_capacity(entries.len() + 2);
            dirents.push((hdr.nodeid, DT_DIR, ".".into()));
            let parent_ino = state.path_to_ino.get(parent_and_name(&dir_rel).0).copied();
            dirents.push((parent_ino.unwrap_or(FUSE_ROOT_ID), DT_DIR, "..".into()));
            for entry in entries {
                let name = entry.name.trim_matches('/');
                if name.is_empty() || name == "." || name == ".." || name.contains('/') {
                    continue;
                }
                let child_rel = join_rel_path(&dir_rel, name);
                let ino = state.intern_path(
                    &child_rel,
                    InodeMeta { is_dir: entry.is_dir, size: entry.size, modified_ms: entry.modified },
                );
                let dt = if entry.is_dir { DT_DIR } else { DT_REG };
                dirents.push((ino, dt, name.to_owned()));
            }
            let packed = pack_fuse_dirents(&dirents, read_in.offset, read_in.size as usize);
            send_fuse_reply(fuse_fd, hdr.unique, 0, &packed);
        }
        FUSE_OPEN => {
            let Some(open_in) = read_struct::<FuseOpenIn>(body) else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EINVAL, &[]);
                return true;
            };
            let mut state = lock(state_arc);
            let Some(rel_path) = state.ino_to_path.get(&hdr.nodeid).cloned() else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::ENOENT, &[]);
                return true;
            };
            let truncate = (open_in.flags as i32 & libc::O_TRUNC) != 0;
            if truncate && let Some(meta) = state.meta_by_ino.get_mut(&hdr.nodeid) {
                meta.size = 0;
            }
            let spool = if truncate {
                match new_spool() {
                    Ok(spool) => Some(spool),
                    Err(err) => {
                        send_fuse_reply(fuse_fd, hdr.unique, err, &[]);
                        return true;
                    }
                }
            } else {
                None
            };
            let fh = state.next_fh;
            state.next_fh += 1;
            state.handles.insert(
                fh,
                OpenFileHandle {
                    nodeid: hdr.nodeid,
                    rel_path,
                    dirty: truncate,
                    spool,
                    read_cache_offset: 0,
                    read_cache_data: Vec::new(),
                },
            );
            let out = FuseOpenOut { fh, open_flags: 0, padding: 0 };
            send_fuse_reply(fuse_fd, hdr.unique, 0, as_bytes(&out));
        }
        FUSE_READ => {
            let Some(read_in) = read_struct::<FuseReadIn>(body) else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EINVAL, &[]);
                return true;
            };
            let want = read_in.size as usize;
            let offset = read_in.offset;
            let state = lock(state_arc);
            let device = state.device;
            if let Some(h) = state.handles.get(&read_in.fh) {
                if let Some(spool) = &h.spool {
                    let read = spool.read_at(offset, want);
                    drop(state);
                    match read {
                        Ok(slice) => send_fuse_reply(fuse_fd, hdr.unique, 0, &slice),
                        Err(e) => send_fuse_reply(fuse_fd, hdr.unique, io_errno(&e), &[]),
                    }
                    return true;
                }
                if offset >= h.read_cache_offset {
                    let rel_off = (offset - h.read_cache_offset) as usize;
                    if rel_off + want <= h.read_cache_data.len() {
                        let slice = h.read_cache_data[rel_off..rel_off + want].to_vec();
                        drop(state);
                        send_fuse_reply(fuse_fd, hdr.unique, 0, &slice);
                        return true;
                    }
                }
            }
            let Some(rel_path) = state
                .handles
                .get(&read_in.fh)
                .map(|h| h.rel_path.clone())
                .or_else(|| state.ino_to_path.get(&hdr.nodeid).cloned())
            else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::ENOENT, &[]);
                return true;
            };
            drop(state);

            let prefetch_len = want.max(256 * 1024);
            match read_remote_slice(device, &rel_path, offset, prefetch_len) {
                Ok(data) => {
                    let ret_len = want.min(data.len());
                    let ret_slice = data[..ret_len].to_vec();
                    let mut state = lock(state_arc);
                    if let Some(h) = state.handles.get_mut(&read_in.fh) {
                        h.read_cache_offset = offset;
                        h.read_cache_data = data;
                    }
                    drop(state);
                    send_fuse_reply(fuse_fd, hdr.unique, 0, &ret_slice);
                }
                Err(err) => send_fuse_reply(fuse_fd, hdr.unique, err, &[]),
            }
        }
        FUSE_CREATE | FUSE_MKNOD => {
            let (name_offset, is_create) = if hdr.opcode == FUSE_CREATE {
                (std::mem::size_of::<FuseCreateIn>(), true)
            } else {
                (std::mem::size_of::<FuseMknodIn>(), false)
            };
            if body.len() <= name_offset {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EINVAL, &[]);
                return true;
            }
            let Some(child_name) = parse_cstr(&body[name_offset..]) else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EINVAL, &[]);
                return true;
            };
            let mut state = lock(state_arc);
            let Some(parent_rel) = state.ino_to_path.get(&hdr.nodeid).cloned() else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::ENOENT, &[]);
                return true;
            };
            let child_rel = join_rel_path(&parent_rel, child_name);
            let now_ms =
                SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
            let meta = InodeMeta { is_dir: false, size: 0, modified_ms: now_ms };
            let ino = state.intern_path(&child_rel, meta.clone());
            let entry = state.entry_out(ino, &meta);

            if is_create {
                let spool = match new_spool() {
                    Ok(spool) => spool,
                    Err(err) => {
                        send_fuse_reply(fuse_fd, hdr.unique, err, &[]);
                        return true;
                    }
                };
                let fh = state.next_fh;
                state.next_fh += 1;
                state.handles.insert(
                    fh,
                    OpenFileHandle {
                        nodeid: ino,
                        rel_path: child_rel,
                        dirty: true,
                        spool: Some(spool),
                        read_cache_offset: 0,
                        read_cache_data: Vec::new(),
                    },
                );
                let open_out = FuseOpenOut { fh, open_flags: 0, padding: 0 };
                let mut payload = Vec::with_capacity(
                    std::mem::size_of::<FuseEntryOut>() + std::mem::size_of::<FuseOpenOut>(),
                );
                payload.extend_from_slice(as_bytes(&entry));
                payload.extend_from_slice(as_bytes(&open_out));
                send_fuse_reply(fuse_fd, hdr.unique, 0, &payload);
            } else {
                let dev = state.device;
                state.invalidate_dir(&parent_rel);
                drop(state);
                let saved = new_spool().and_then(|empty| upload_file(dev, &child_rel, &empty.path));
                if let Err(err) = saved {
                    send_fuse_reply(fuse_fd, hdr.unique, err, &[]);
                    return true;
                }
                send_fuse_reply(fuse_fd, hdr.unique, 0, as_bytes(&entry));
            }
        }
        FUSE_WRITE => {
            let Some(write_in) = read_struct::<FuseWriteIn>(body) else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EINVAL, &[]);
                return true;
            };
            let data_offset = std::mem::size_of::<FuseWriteIn>();
            let write_len = write_in.size as usize;
            if body.len() < data_offset + write_len {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EINVAL, &[]);
                return true;
            }
            let write_bytes = &body[data_offset..data_offset + write_len];

            if let Err(err) = load_for_writing(state_arc, write_in.fh) {
                send_fuse_reply(fuse_fd, hdr.unique, err, &[]);
                return true;
            }
            let mut state = lock(state_arc);
            let Some(h) = state.handles.get_mut(&write_in.fh) else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EBADF, &[]);
                return true;
            };
            let Some(spool) = h.spool.as_mut() else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EBADF, &[]);
                return true;
            };
            if let Err(e) = spool.write_at(write_in.offset, write_bytes) {
                send_fuse_reply(fuse_fd, hdr.unique, io_errno(&e), &[]);
                return true;
            }
            let new_len = spool.len;
            h.dirty = true;
            let ino = h.nodeid;
            if let Some(meta) = state.meta_by_ino.get_mut(&ino) {
                meta.size = new_len;
            }
            let out = FuseWriteOut { size: write_in.size, padding: 0 };
            send_fuse_reply(fuse_fd, hdr.unique, 0, as_bytes(&out));
        }
        FUSE_FLUSH | FUSE_FSYNC => {
            let fh = if hdr.opcode == FUSE_FLUSH {
                read_struct::<FuseFlushIn>(body).map(|f| f.fh)
            } else {
                read_struct::<FuseFsyncIn>(body).map(|f| f.fh)
            };
            let Some(fh) = fh else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EINVAL, &[]);
                return true;
            };
            let mut state = lock(state_arc);
            let dev = state.device;
            let pending = state.handles.get_mut(&fh).and_then(|h| {
                let path = h.spool.as_ref().filter(|_| h.dirty)?.path.clone();
                h.dirty = false;
                Some((dev, h.rel_path.clone(), path))
            });
            drop(state);
            // Requests are handled one at a time, so nothing writes to the spool
            // while it uploads.
            if let Some((dev, rel_path, path)) = pending {
                if let Err(err) = upload_file(dev, &rel_path, &path) {
                    let mut st = lock(state_arc);
                    if let Some(h) = st.handles.get_mut(&fh) {
                        h.dirty = true;
                    }
                    send_fuse_reply(fuse_fd, hdr.unique, err, &[]);
                    return true;
                }
                let (parent_rel, _) = parent_and_name(&rel_path);
                lock(state_arc).invalidate_dir(parent_rel);
            }
            send_fuse_reply(fuse_fd, hdr.unique, 0, &[]);
        }
        FUSE_RELEASE => {
            let Some(rel_in) = read_struct::<FuseReleaseIn>(body) else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EINVAL, &[]);
                return true;
            };
            let mut state = lock(state_arc);
            let removed = state.handles.remove(&rel_in.fh);
            let dev = state.device;
            drop(state);
            if let Some(h) = removed
                && h.dirty
                && let Some(spool) = &h.spool
            {
                // Nothing can receive an error by now: the file has been closed.
                if let Err(err) = upload_file(dev, &h.rel_path, &spool.path) {
                    tracing::warn!(err, "couldn't save a file to the phone");
                }
                let (parent_rel, _) = parent_and_name(&h.rel_path);
                lock(state_arc).invalidate_dir(parent_rel);
            }
            send_fuse_reply(fuse_fd, hdr.unique, 0, &[]);
        }
        FUSE_MKDIR => {
            let name_offset = std::mem::size_of::<FuseMkdirIn>();
            if body.len() <= name_offset {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EINVAL, &[]);
                return true;
            }
            let Some(dir_name) = parse_cstr(&body[name_offset..]) else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EINVAL, &[]);
                return true;
            };
            let (dev, parent_rel, child_rel) = {
                let state = lock(state_arc);
                let Some(parent_rel) = state.ino_to_path.get(&hdr.nodeid).cloned() else {
                    send_fuse_reply(fuse_fd, hdr.unique, -libc::ENOENT, &[]);
                    return true;
                };
                let child_rel = join_rel_path(&parent_rel, dir_name);
                (state.device, parent_rel, child_rel)
            };
            let Some(node) = core_host::node() else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::ENOTCONN, &[]);
                return true;
            };
            let child_clone = child_rel.clone();
            let res = core_host::host()
                .runtime
                .block_on(async move { node.storage_mkdir(dev, &child_clone).await });
            if res.is_err() {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EIO, &[]);
                return true;
            }
            let now_ms =
                SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
            let mut state = lock(state_arc);
            state.invalidate_dir(&parent_rel);
            let meta = InodeMeta { is_dir: true, size: 4096, modified_ms: now_ms };
            let ino = state.intern_path(&child_rel, meta.clone());
            let entry = state.entry_out(ino, &meta);
            send_fuse_reply(fuse_fd, hdr.unique, 0, as_bytes(&entry));
        }
        FUSE_UNLINK | FUSE_RMDIR => {
            let Some(child_name) = parse_cstr(body) else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EINVAL, &[]);
                return true;
            };
            let (dev, parent_rel, child_rel) = {
                let state = lock(state_arc);
                let Some(parent_rel) = state.ino_to_path.get(&hdr.nodeid).cloned() else {
                    send_fuse_reply(fuse_fd, hdr.unique, -libc::ENOENT, &[]);
                    return true;
                };
                let child_rel = join_rel_path(&parent_rel, child_name);
                (state.device, parent_rel, child_rel)
            };
            let Some(node) = core_host::node() else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::ENOTCONN, &[]);
                return true;
            };
            let child_clone = child_rel.clone();
            let res = core_host::host()
                .runtime
                .block_on(async move { node.storage_delete(dev, &child_clone, true).await });
            if res.is_err() {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EIO, &[]);
                return true;
            }
            let mut state = lock(state_arc);
            state.invalidate_dir(&parent_rel);
            if let Some(ino) = state.path_to_ino.remove(&child_rel) {
                state.ino_to_path.remove(&ino);
                state.meta_by_ino.remove(&ino);
            }
            send_fuse_reply(fuse_fd, hdr.unique, 0, &[]);
        }
        FUSE_RENAME | FUSE_RENAME2 => {
            let (newdir, names_offset) = if hdr.opcode == FUSE_RENAME {
                let Some(r) = read_struct::<FuseRenameIn>(body) else {
                    send_fuse_reply(fuse_fd, hdr.unique, -libc::EINVAL, &[]);
                    return true;
                };
                (r.newdir, std::mem::size_of::<FuseRenameIn>())
            } else {
                let Some(r) = read_struct::<FuseRename2In>(body) else {
                    send_fuse_reply(fuse_fd, hdr.unique, -libc::EINVAL, &[]);
                    return true;
                };
                (r.newdir, std::mem::size_of::<FuseRename2In>())
            };
            if body.len() <= names_offset {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EINVAL, &[]);
                return true;
            }
            let Some((old_name, new_name)) = parse_two_cstrs(&body[names_offset..]) else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EINVAL, &[]);
                return true;
            };
            let (dev, old_parent, new_parent, old_rel, new_rel) = {
                let state = lock(state_arc);
                let Some(old_parent) = state.ino_to_path.get(&hdr.nodeid).cloned() else {
                    send_fuse_reply(fuse_fd, hdr.unique, -libc::ENOENT, &[]);
                    return true;
                };
                let Some(new_parent) = state.ino_to_path.get(&newdir).cloned() else {
                    send_fuse_reply(fuse_fd, hdr.unique, -libc::ENOENT, &[]);
                    return true;
                };
                let old_rel = join_rel_path(&old_parent, old_name);
                let new_rel = join_rel_path(&new_parent, new_name);
                (state.device, old_parent, new_parent, old_rel, new_rel)
            };
            let Some(node) = core_host::node() else {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::ENOTCONN, &[]);
                return true;
            };
            let old_clone = old_rel.clone();
            let new_clone = new_rel.clone();
            let res = core_host::host()
                .runtime
                .block_on(async move { node.storage_rename(dev, &old_clone, &new_clone).await });
            if res.is_err() {
                send_fuse_reply(fuse_fd, hdr.unique, -libc::EIO, &[]);
                return true;
            }
            let mut state = lock(state_arc);
            state.invalidate_dir(&old_parent);
            state.invalidate_dir(&new_parent);
            state.rename_path(&old_rel, &new_rel);
            send_fuse_reply(fuse_fd, hdr.unique, 0, &[]);
        }
        _ => {
            send_fuse_reply(fuse_fd, hdr.unique, -libc::ENOSYS, &[]);
        }
    }
    true
}

fn mount_worker(mounted: &Arc<MountedDevice>) {
    if mounted.stop.load(Ordering::Acquire) {
        return;
    }
    let Some(fuse_fd) = open_fuse_mount(&mounted.mount_path) else {
        tracing::warn!(
            path = %mounted.mount_path.display(),
            "FUSE mount unavailable (ensure /dev/fuse and fusermount3 are installed)"
        );
        *lock(&mounted.died_at) = Some(Instant::now());
        return;
    };

    let raw_fd = fuse_fd.as_raw_fd();
    let mut buf = vec![0u8; MAX_FUSE_MSG_LEN];
    let hdr_size = std::mem::size_of::<FuseInHeader>();

    while !mounted.stop.load(Ordering::Acquire) {
        let mut pfd = [libc::pollfd { fd: raw_fd, events: libc::POLLIN, revents: 0 }];
        // SAFETY: `pfd` is a valid 1-element `pollfd` array.
        let prc = unsafe { libc::poll(pfd.as_mut_ptr(), 1, 500) };
        if mounted.stop.load(Ordering::Acquire) {
            break;
        }
        if prc < 0 {
            let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if err == libc::EINTR {
                continue;
            }
            break;
        }
        if prc == 0 || (pfd[0].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
            if (pfd[0].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
                break;
            }
            continue;
        }
        if (pfd[0].revents & libc::POLLIN) == 0 {
            continue;
        }

        // SAFETY: `buf` is a valid mutable heap buffer and `raw_fd` is our open `/dev/fuse` descriptor.
        let n = unsafe { libc::read(raw_fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if err == libc::EINTR || err == libc::EAGAIN {
                continue;
            }
            // ENODEV means the filesystem was unmounted.
            break;
        }
        let n = n as usize;
        if n < hdr_size {
            continue;
        }
        let Some(hdr) = read_struct::<FuseInHeader>(&buf[..hdr_size]) else {
            continue;
        };
        let body = &buf[hdr_size..n];
        if !handle_fuse_request(raw_fd, &mounted.fuse_state, hdr, body) {
            break;
        }
    }

    drop(fuse_fd);
    unmount_fuse_path(&mounted.mount_path);
    *lock(&mounted.died_at) = Some(Instant::now());
}

pub fn start() {
    SHUTTING_DOWN.store(false, Ordering::Release);
    drop(mounts());
    // Unsaved files from a run that didn't get to clean up.
    let _ = fs::remove_dir_all(core_host::host().data_dir.join("fuse-stage"));
    core_host::host().hub.subscribe(
        Changes::DEVICES | Changes::STATUS | Changes::CAPABILITIES | Changes::TOGGLES,
        || {
            sync();
            true
        },
    );
    sync();
}

pub fn sync() {
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        return;
    }
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
    let mut folders = folder_names(devices.iter().map(|(id, name, _)| (*id, name.as_str())));
    let mut desired: Vec<DesiredMount> = Vec::new();
    let mut disabled: Vec<(DeviceId, String)> = Vec::new();
    for (id, name, online) in devices {
        // Not being able to read the setting isn't the setting being off: leave
        // the mount and its open files alone.
        let Ok(toggles) = node.device_toggles(id) else {
            tracing::debug!(device = %id.short(), "can't read storage permission; skipping this sync");
            return;
        };
        let enabled = toggles.into_iter().find(|(k, _)| *k == "storage").is_some_and(|(_, on)| on);
        let folder = folders.remove(&id).unwrap_or_else(|| sanitize_folder_name(&name));
        if enabled {
            desired.push(DesiredMount { device: id, name, folder, online });
        } else {
            disabled.push((id, folder));
        }
    }

    let _ = std::thread::Builder::new().name("storage-linux-reconcile".into()).spawn(move || {
        let _guard = lock(&LINUX_RECONCILING);
        reconcile(&desired, &disabled);
    });
}

struct DesiredMount {
    device: DeviceId,
    name: String,
    folder: String,
    online: bool,
}

fn reconcile(desired: &[DesiredMount], disabled: &[(DeviceId, String)]) {
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        return;
    }
    let Some(base) = base_sync_dir() else { return };

    for (id, folder) in disabled {
        unmount_device(*id, Some(folder), true);
    }

    let desired_ids: HashSet<DeviceId> = desired.iter().map(|d| d.device).collect();
    let stale: Vec<DeviceId> = mounts().keys().filter(|id| !desired_ids.contains(id)).copied().collect();
    for id in stale {
        unmount_device(id, None, true);
    }

    for DesiredMount { device, name, folder, online } in desired {
        let mount_path = base.join(folder);
        let existing = get_mounted_device(device);
        if existing.as_ref().is_none_or(|m| m.retry_due()) {
            clear_stale_mount(&mount_path);
        }
        if let Err(e) = fs::create_dir_all(&mount_path) {
            tracing::warn!(error = %e, "can't create the phone storage folder");
            continue;
        }
        register_sidebar_bookmarks(&mount_path, name);

        if let Some(mounted) = existing {
            let unchanged = mounted.mount_path == mount_path && mounted.device_name == *name;
            if unchanged && !mounted.retry_due() {
                if *online {
                    mounted.invalidate_cache();
                }
                continue;
            }
            unmount_device(*device, None, !unchanged);
        }

        let stop = Arc::new(AtomicBool::new(false));
        let fuse_state = Arc::new(Mutex::new(FuseState::new(*device)));
        let mounted = Arc::new(MountedDevice {
            device: *device,
            device_name: name.clone(),
            mount_path,
            stop,
            fuse_state,
            worker: Mutex::new(None),
            died_at: Mutex::new(None),
        });
        let worker_mount = Arc::clone(&mounted);
        if let Ok(handle) = std::thread::Builder::new()
            .name(format!("storage-fuse-{}", &device.to_string()[..8]))
            .spawn(move || mount_worker(&worker_mount))
        {
            *lock(&mounted.worker) = Some(handle);
            mounts().insert(*device, mounted);
        }
    }
}

fn unmount_device(device: DeviceId, fallback_folder: Option<&str>, remove_bookmarks: bool) {
    let removed = mounts().remove(&device);
    if let Some(mounted) = removed {
        mounted.stop_and_join();
        if remove_bookmarks {
            unregister_sidebar_bookmarks(&mounted.mount_path);
        }
    } else if remove_bookmarks
        && let Some(folder) = fallback_folder
        && let Some(base) = base_sync_dir()
    {
        let mount_path = base.join(folder);
        unregister_sidebar_bookmarks(&mount_path);
    }
}

pub fn on_event(event: &NodeEvent) {
    match event {
        NodeEvent::DeviceRemoved(device) => {
            let device = *device;
            let _ = std::thread::Builder::new().name("storage-linux-unpair".into()).spawn(move || {
                let _guard = lock(&LINUX_RECONCILING);
                unmount_device(device, None, true);
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
        NodeEvent::StorageChanged { device, .. } => {
            if let Some(mounted) = get_mounted_device(device) {
                mounted.invalidate_cache();
            }
        }
        _ => {}
    }
}

pub fn shutdown() {
    SHUTTING_DOWN.store(true, Ordering::Release);
    let _guard = lock(&LINUX_RECONCILING);
    let active: Vec<Arc<MountedDevice>> = mounts().drain().map(|(_, v)| v).collect();
    for mounted in active {
        mounted.stop_and_join();
    }
}

pub fn unregister_all() {
    shutdown();
    if let Some(bookmarks_file) = gtk_bookmarks_path()
        && let Ok(existing) = fs::read_to_string(&bookmarks_file)
    {
        let filtered: Vec<&str> = existing
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty() && !line.ends_with(" (NectarLink)"))
            .collect();
        let updated = if filtered.is_empty() { String::new() } else { format!("{}\n", filtered.join("\n")) };
        let _ = fs::write(&bookmarks_file, updated);
    }
    if let Some(xbel_file) = xbel_bookmarks_path()
        && let Ok(existing) = fs::read_to_string(&xbel_file)
    {
        let updated = remove_xbel_blocks_matching(&existing, |b| {
            b.contains("id=\"nectarlink\"") || b.contains("(NectarLink)</title>")
        });
        let _ = fs::write(&xbel_file, updated);
    }
    if let Some(base) = base_sync_dir() {
        let _ = fs::remove_dir(&base);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_failures_map_to_distinct_errnos() {
        use nectarlink_core::Error;
        assert_eq!(storage_errno(&Error::Offline), -libc::ENOTCONN);
        assert_eq!(storage_errno(&Error::Denied), -libc::EACCES);
        assert_eq!(storage_errno(&Error::NotFound), -libc::ENOENT);
        assert_eq!(storage_errno(&Error::Timeout), -libc::ETIMEDOUT);
        assert_eq!(storage_errno(&Error::Internal("x".into())), -libc::EIO);
        let io = Error::Io(std::io::Error::from_raw_os_error(libc::ENOSPC));
        assert_eq!(storage_errno(&io), -libc::ENOSPC);
    }

    #[test]
    fn gtk_bookmark_formatting_upsert_and_removal_preserve_unrelated_entries() {
        let mount = Path::new("/home/alice/Nectarlink/Pixel 9 Pro");
        let line = format_gtk_bookmark_line(mount, "Pixel 9 Pro");
        assert_eq!(line, "file:///home/alice/Nectarlink/Pixel%209%20Pro Pixel 9 Pro (NectarLink)");

        let initial = "file:///home/alice/Documents Documents\nfile:///home/alice/Downloads Downloads\n";
        let with_phone = upsert_gtk_bookmarks_content(initial, mount, "Pixel 9 Pro");
        assert!(with_phone.contains("file:///home/alice/Documents Documents\n"));
        assert!(with_phone.contains("file:///home/alice/Downloads Downloads\n"));
        assert!(with_phone.contains(&line));

        // Upserting again with updated label replaces in-place without duplicating.
        let updated_phone = upsert_gtk_bookmarks_content(&with_phone, mount, "Pixel 9 Pro Renamed");
        assert_eq!(
            updated_phone
                .lines()
                .filter(|l| l.starts_with("file:///home/alice/Nectarlink/Pixel%209%20Pro"))
                .count(),
            1
        );
        assert!(updated_phone.contains("Pixel 9 Pro Renamed (NectarLink)"));

        let cleaned = remove_gtk_bookmarks_content(&updated_phone, mount);
        assert_eq!(cleaned, initial);
    }

    #[test]
    fn xbel_bookmark_upsert_and_removal_preserve_user_places() {
        let mount = Path::new("/home/alice/Nectarlink/Pixel 9 Pro");
        let initial = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE xbel>\n<xbel xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\">\n <bookmark href=\"file:///home/alice/Downloads\"><title>Downloads</title></bookmark>\n</xbel>\n";
        let with_phone = upsert_xbel_bookmarks_content(initial, mount, "Pixel 9 Pro");
        assert!(with_phone.contains("href=\"file:///home/alice/Downloads\""));
        assert!(with_phone.contains("href=\"file:///home/alice/Nectarlink/Pixel%209%20Pro\""));
        assert!(with_phone.contains("<bookmark:icon name=\"smartphone\"/>"));

        // Upsert is idempotent.
        let again = upsert_xbel_bookmarks_content(&with_phone, mount, "Pixel 9 Pro");
        assert_eq!(again.matches("href=\"file:///home/alice/Nectarlink/Pixel%209%20Pro\"").count(), 1);

        let cleaned = remove_xbel_bookmarks_content(&again, mount);
        assert!(cleaned.contains("href=\"file:///home/alice/Downloads\""));
        assert!(!cleaned.contains("Pixel%209%20Pro"));
    }

    #[test]
    fn packs_fuse_dirents_with_8_byte_alignment_and_offset_slicing() {
        let entries = vec![
            (1u64, DT_DIR, ".".to_string()),
            (1u64, DT_DIR, "..".to_string()),
            (2u64, DT_DIR, "DCIM".to_string()),
            (3u64, DT_REG, "photo.jpg".to_string()),
        ];
        let all = pack_fuse_dirents(&entries, 0, 4096);
        // "." (24+1 -> 32) + ".." (24+2 -> 32) + "DCIM" (24+4 -> 32) + "photo.jpg" (24+9 -> 40) = 136 bytes.
        assert_eq!(all.len(), 136);
        assert_eq!(all.len() % 8, 0);

        // Starting from offset 2 skips "." and "..".
        let from_two = pack_fuse_dirents(&entries, 2, 4096);
        assert_eq!(from_two.len(), 72);

        // Verify FUSE ABI struct sizes match `<linux/fuse.h>`.
        assert_eq!(std::mem::size_of::<FuseInHeader>(), 40);
        assert_eq!(std::mem::size_of::<FuseOutHeader>(), 16);
        assert_eq!(std::mem::size_of::<FuseAttr>(), 88);
        assert_eq!(std::mem::size_of::<FuseEntryOut>(), 128);
        assert_eq!(std::mem::size_of::<FuseAttrOut>(), 104);
    }

    #[test]
    fn to_remote_rel_path_converts_linux_mount_paths_to_slash_relative_paths() {
        let root = Path::new("/home/alice/Nectarlink/Pixel 9");
        assert_eq!(to_remote_rel_path(root, root), Some(String::new()));
        assert_eq!(
            to_remote_rel_path(root, Path::new("/home/alice/Nectarlink/Pixel 9/DCIM/Camera/IMG_001.jpg")),
            Some("DCIM/Camera/IMG_001.jpg".to_owned())
        );
        assert_eq!(to_remote_rel_path(root, Path::new("/home/alice/Nectarlink/OtherPhone/DCIM")), None);
    }

    #[test]
    fn finds_a_mount_left_by_an_earlier_run() {
        let mounts = "sysfs /sys sysfs rw 0 0\n\
                      nectarlink /home/bee/Nectarlink/Pixel\\0409 fuse.nectarlink rw,nosuid,nodev 0 0\n\
                      portal /run/user/1000/doc fuse.portal rw 0 0\n";
        assert!(is_listed_mount(mounts, Path::new("/home/bee/Nectarlink/Pixel 9")));
        assert!(!is_listed_mount(mounts, Path::new("/home/bee/Nectarlink/Galaxy")));
        assert!(!is_listed_mount(mounts, Path::new("/run/user/1000/doc")), "only NectarLink's own mounts");
    }

    fn test_spool_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nl-spool-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_spool_reads_back_writes_and_resizes_without_holding_the_file_in_memory() {
        let dir = test_spool_dir("rw");
        let mut spool = Spool::create(&dir).unwrap();
        spool.write_at(0, b"hello").unwrap();
        spool.write_at(10, b"world").unwrap();
        assert_eq!(spool.len, 15);
        assert_eq!(spool.read_at(3, 4).unwrap(), b"lo\0\0");
        assert_eq!(spool.read_at(13, 100).unwrap(), b"ld", "stops at the end");
        assert!(spool.read_at(99, 10).unwrap().is_empty(), "past the end");
        spool.set_len(2).unwrap();
        assert_eq!(spool.read_at(0, 100).unwrap(), b"he");
        // Far past any PC's memory, and it costs nothing until written.
        spool.set_len(1 << 40).unwrap();
        assert_eq!(spool.read_at((1 << 40) - 1, 8).unwrap(), [0]);
        let path = spool.path.clone();
        assert!(path.exists());
        drop(spool);
        assert!(!path.exists(), "dropped with its handle");
        let _ = fs::remove_dir_all(&dir);
    }

    fn handle(nodeid: u64, rel_path: &str) -> OpenFileHandle {
        OpenFileHandle {
            nodeid,
            rel_path: rel_path.to_owned(),
            dirty: false,
            spool: None,
            read_cache_offset: 0,
            read_cache_data: Vec::new(),
        }
    }

    fn file_meta() -> InodeMeta {
        InodeMeta { is_dir: false, size: 1, modified_ms: 0 }
    }

    fn test_state() -> FuseState {
        FuseState::new(DeviceId([1; 32]))
    }

    #[test]
    fn renaming_a_folder_moves_what_is_inside_it() {
        let mut state = test_state();
        let folder = state.intern_path("DCIM", file_meta());
        let inside = state.intern_path("DCIM/Camera/a.jpg", file_meta());
        let lookalike = state.intern_path("DCIM2/b.jpg", file_meta());
        state.handles.insert(1, handle(inside, "DCIM/Camera/a.jpg"));

        state.rename_path("DCIM", "Pictures");

        assert_eq!(state.ino_to_path[&folder], "Pictures");
        assert_eq!(state.ino_to_path[&inside], "Pictures/Camera/a.jpg");
        assert_eq!(state.path_to_ino["Pictures/Camera/a.jpg"], inside);
        assert_eq!(state.handles[&1].rel_path, "Pictures/Camera/a.jpg");
        assert_eq!(state.ino_to_path[&lookalike], "DCIM2/b.jpg", "only that folder");
        assert!(!state.path_to_ino.contains_key("DCIM"));
    }

    #[test]
    fn renaming_over_a_file_forgets_the_old_file() {
        let mut state = test_state();
        let moved = state.intern_path("new.tmp", file_meta());
        let replaced = state.intern_path("note.txt", file_meta());

        state.rename_path("new.tmp", "note.txt");

        assert_eq!(state.path_to_ino["note.txt"], moved);
        assert!(!state.ino_to_path.contains_key(&replaced));
    }

    #[test]
    fn unsaved_writes_decide_the_reported_size() {
        let dir = test_spool_dir("len");
        let mut state = test_state();
        let ino = state.intern_path("a.bin", file_meta());
        assert_eq!(state.unsaved_len(ino), None);
        let mut spool = Spool::create(&dir).unwrap();
        spool.write_at(0, &[0; 9]).unwrap();
        let mut open = handle(ino, "a.bin");
        open.spool = Some(spool);
        open.dirty = true;
        state.handles.insert(1, open);
        assert_eq!(state.unsaved_len(ino), Some(9));
        state.handles.get_mut(&1).unwrap().dirty = false;
        assert_eq!(state.unsaved_len(ino), None, "saved: the phone knows");
        state.handles.clear();
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitizes_device_folder_names() {
        assert_eq!(sanitize_folder_name("Pixel 9 Pro"), "Pixel 9 Pro");
        assert!(!sanitize_folder_name("../bad/name").contains('/'));
    }

    #[test]
    fn maintains_gtk_bookmarks_idempotently_and_preserves_user_entries() {
        let mount = Path::new("/home/alice/Nectarlink/Galaxy S24");
        let existing = "file:///home/alice/Music Music\n";
        let once = upsert_gtk_bookmarks_content(existing, mount, "Galaxy S24");
        let twice = upsert_gtk_bookmarks_content(&once, mount, "Galaxy S24");
        assert_eq!(once, twice);
        assert_eq!(remove_gtk_bookmarks_content(&twice, mount), existing);
    }

    #[test]
    fn normalize_rel_path_rejects_parent_traversal() {
        let root = Path::new("/home/alice/Nectarlink/Pixel 9");
        assert_eq!(to_remote_rel_path(root, Path::new("/home/alice/Nectarlink/Pixel 9/../secret")), None);
        assert_eq!(parse_cstr(b"..\0"), None);
        assert_eq!(parse_cstr(b".\0"), None);
        assert_eq!(parse_cstr(b"sub/dir\0"), None);
        assert_eq!(parse_cstr(b"photo.jpg\0"), Some("photo.jpg"));
    }
}
