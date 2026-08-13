//! Stable, no-follow capture for read-only regular-file inputs.

use std::{
    fs::{self, File, Metadata},
    io,
    path::{Path, PathBuf},
};

use crate::publication::same_file_identity;

#[derive(Debug)]
pub(crate) struct StableFile {
    path: PathBuf,
    file: File,
    identity: Metadata,
}

impl StableFile {
    pub(crate) fn capture(path: &Path) -> io::Result<Self> {
        let inspected = fs::symlink_metadata(path)?;
        Self::capture_inspected(path, &inspected)
    }

    pub(crate) fn byte_length(&self) -> u64 {
        self.identity.len()
    }

    pub(crate) fn file_mut(&mut self) -> &mut File {
        &mut self.file
    }

    pub(crate) fn verify(&self) -> io::Result<()> {
        let opened = self.file.metadata()?;
        let current = fs::symlink_metadata(&self.path)?;
        require_same_file_state(&self.identity, &opened, &current, "captured input changed")
    }

    fn capture_inspected(path: &Path, inspected: &Metadata) -> io::Result<Self> {
        require_regular(inspected)?;
        require_stable_identity(inspected)?;
        let file = open_no_follow(path)?;
        #[cfg(windows)]
        require_disk_file(&file)?;
        let opened = file.metadata()?;
        let current = fs::symlink_metadata(path)?;
        require_same_file_state(
            inspected,
            &opened,
            &current,
            "input changed while it was being opened",
        )?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            identity: opened,
        })
    }

    #[cfg(test)]
    pub(crate) fn capture_from_metadata(path: &Path, inspected: &Metadata) -> io::Result<Self> {
        Self::capture_inspected(path, inspected)
    }
}

fn require_same_file_state(
    initial: &Metadata,
    opened: &Metadata,
    current: &Metadata,
    message: &'static str,
) -> io::Result<()> {
    require_regular(opened)?;
    require_regular(current)?;
    require_stable_identity(opened)?;
    require_stable_identity(current)?;
    if same_state(initial, opened) && same_state(opened, current) {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidData, message))
    }
}

fn require_regular(metadata: &Metadata) -> io::Result<()> {
    if metadata.file_type().is_file() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "input must be a regular file and not a symbolic link",
        ))
    }
}

fn require_stable_identity(metadata: &Metadata) -> io::Result<()> {
    if same_file_identity(metadata, metadata) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "filesystem does not expose a stable file identity",
        ))
    }
}

#[cfg(unix)]
fn same_state(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    same_file_identity(left, right)
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

#[cfg(windows)]
fn same_state(left: &Metadata, right: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;

    same_file_identity(left, right)
        && left.len() == right.len()
        && left.creation_time() == right.creation_time()
        && left.last_write_time() == right.last_write_time()
}

#[cfg(not(any(unix, windows)))]
fn same_state(_left: &Metadata, _right: &Metadata) -> bool {
    false
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(windows)]
fn open_no_follow(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt as _;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_no_follow(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "stable no-follow input capture is unavailable on this platform",
    ))
}

#[cfg(windows)]
fn require_disk_file(file: &File) -> io::Result<()> {
    let file_type = winapi_util::file::typ(file)?;
    if file_type.is_disk() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "input must use a disk-backed regular file",
        ))
    }
}
