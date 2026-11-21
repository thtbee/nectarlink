// SPDX-License-Identifier: GPL-3.0-or-later
use std::{
    fs::{File, Permissions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, PermissionsExt},
    },
    path::{Component, Path, PathBuf},
};

use nectarlink_protocol::DeviceId;
use ring::{agreement, rand::SystemRandom};
use rustix::{
    fs::{AtFlags, Mode, OFlags, open, openat, unlinkat},
    process::{geteuid, getuid},
};
use serde::{Deserialize, Serialize};

pub const DEFAULT_ENROLLMENT_DIR: &str = "/etc/nectarlink/phone-unlock";
const MAX_ENROLLMENT_BYTES: u64 = 4096;
const DIRECTORY_FLAGS: OFlags =
    OFlags::RDONLY.union(OFlags::DIRECTORY).union(OFlags::NOFOLLOW).union(OFlags::CLOEXEC);

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Enrollment {
    pub pc: DeviceId,
    pub phone: DeviceId,
    pub uid: u32,
    pub public_key: Vec<u8>,
}

impl Enrollment {
    pub fn read(input: impl Read) -> io::Result<Self> {
        let mut bytes = Vec::new();
        input.take(MAX_ENROLLMENT_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_ENROLLMENT_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "enrollment too large"));
        }
        let enrollment: Self = serde_json::from_slice(&bytes)?;
        if enrollment.uid == 0
            || enrollment.pc == enrollment.phone
            || enrollment.public_key.len() != 65
            || enrollment.public_key.first() != Some(&4)
        {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid enrollment"));
        }
        Ok(enrollment)
    }

    pub fn fingerprint(&self) -> String {
        ring::digest::digest(&ring::digest::SHA256, &self.public_key)
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

#[derive(Debug)]
pub struct EnrollmentStore(File);

#[derive(Debug)]
pub(crate) struct Generation {
    // Keep the inode allocated until the transaction ends.
    _file: File,
    device: u64,
    inode: u64,
    change: (i64, i64),
}

impl PartialEq for Generation {
    fn eq(&self, other: &Self) -> bool {
        (self.device, self.inode, self.change) == (other.device, other.inode, other.change)
    }
}

impl EnrollmentStore {
    pub fn open(path: &Path) -> io::Result<Self> {
        open_trusted_directory(path, 0, 0o7022).map(Self)
    }

    pub(crate) fn load(&self, uid: u32) -> io::Result<(Enrollment, Generation)> {
        let file = File::from(openat(
            &self.0,
            format!("{uid}.json"),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )?);
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != 0
            || metadata.mode() & 0o7133 != 0
        {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "untrusted enrollment file"));
        }
        let enrollment = Enrollment::read(&file)?;
        if enrollment.uid != uid {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "enrollment UID mismatch"));
        }
        let generation = Generation {
            _file: file,
            device: metadata.dev(),
            inode: metadata.ino(),
            change: (metadata.ctime(), metadata.ctime_nsec()),
        };
        Ok((enrollment, generation))
    }

    pub fn enroll(&self, enrollment: &Enrollment, confirmed_fingerprint: &str) -> io::Result<()> {
        require_root()?;
        let bytes = serde_json::to_vec(enrollment)?;
        Enrollment::read(bytes.as_slice())?;
        if !enrollment.fingerprint().eq_ignore_ascii_case(confirmed_fingerprint) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "phone fingerprint mismatch"));
        }
        let rng = SystemRandom::new();
        let ephemeral = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng)
            .map_err(|_| io::Error::other("P-256 random generation failed"))?;
        // ring validates that the submitted public point belongs to P-256 during agreement.
        let public_key = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, &enrollment.public_key);
        agreement::agree_ephemeral(ephemeral, &public_key, |_| ())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid P-256 public key"))?;

        let directory = descriptor_path(&self.0);
        let mut temp = tempfile::NamedTempFile::new_in(&directory)?;
        temp.write_all(&bytes)?;
        temp.as_file().set_permissions(Permissions::from_mode(0o644))?;
        temp.as_file().sync_all()?;
        temp.persist(directory.join(format!("{}.json", enrollment.uid)))?;
        self.0.sync_all()
    }

    pub fn revoke(&self, uid: u32) -> io::Result<()> {
        require_root()?;
        match unlinkat(&self.0, format!("{uid}.json"), AtFlags::empty()) {
            Ok(()) => self.0.sync_all(),
            Err(rustix::io::Errno::NOENT) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

pub fn require_root() -> io::Result<()> {
    if getuid().is_root() && geteuid().is_root() {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "run as an administrator"))
    }
}

pub(crate) fn open_trusted_directory(path: &Path, uid: u32, forbidden_mode: u32) -> io::Result<File> {
    let mut components = path.components().peekable();
    if components.next() != Some(Component::RootDir) || components.peek().is_none() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "expected an absolute directory path"));
    }
    let mut directory = File::from(open("/", DIRECTORY_FLAGS, Mode::empty())?);
    check_directory(&directory, 0, 0o7022)?;
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid directory path"));
        };
        directory = File::from(openat(&directory, name, DIRECTORY_FLAGS, Mode::empty())?);
        if components.peek().is_some() {
            check_directory(&directory, 0, 0o7022)?;
        } else {
            check_directory(&directory, uid, forbidden_mode)?;
        }
    }
    Ok(directory)
}

fn check_directory(directory: &File, uid: u32, forbidden_mode: u32) -> io::Result<()> {
    let metadata = directory.metadata()?;
    if metadata.uid() == uid && metadata.mode() & forbidden_mode == 0 {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "untrusted directory ownership or mode"))
    }
}

pub(crate) fn descriptor_path(file: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
}
