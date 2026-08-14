use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub(crate) enum StageCreationError {
    NamespaceExhausted,
    Inspect { path: PathBuf, source: io::Error },
    Create { path: PathBuf, source: io::Error },
}

const MAX_NAMED_STAGES_PER_NAMESPACE: u8 = 64;

pub(crate) fn create_stage<E>(
    parent: &Path,
    namespace: &'static str,
    mut before_attempt: impl FnMut() -> Result<(), E>,
    mut map_error: impl FnMut(StageCreationError) -> E,
) -> Result<(StageGuard, File), E> {
    #[cfg(target_os = "linux")]
    {
        before_attempt()?;
        let display = parent.join(format!(".punctra-{namespace}-unnamed.tmp"));
        let directory = File::open(parent).map_err(|source| {
            map_error(StageCreationError::Create {
                path: display.clone(),
                source,
            })
        })?;
        use rustix::fs::{Mode, OFlags, openat};
        let descriptor = openat(
            &directory,
            ".",
            OFlags::TMPFILE | OFlags::RDWR | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|source| {
            map_error(StageCreationError::Create {
                path: display.clone(),
                source: source.into(),
            })
        })?;
        let file = File::from(descriptor);
        let metadata = file.metadata().map_err(|source| {
            map_error(StageCreationError::Inspect {
                path: display.clone(),
                source,
            })
        })?;
        let source = file.try_clone().map_err(|source| {
            map_error(StageCreationError::Inspect {
                path: display.clone(),
                source,
            })
        })?;
        return Ok((StageGuard::new(display, None, metadata, source), file));
    }
    #[cfg(not(target_os = "linux"))]
    for slot in 0..MAX_NAMED_STAGES_PER_NAMESPACE {
        before_attempt()?;
        let stage = parent.join(format!(".punctra-{namespace}-{slot:02}.tmp"));
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        match options.open(&stage) {
            Ok(file) => {
                let metadata = file.metadata().map_err(|source| {
                    map_error(StageCreationError::Inspect {
                        path: stage.clone(),
                        source,
                    })
                })?;
                let source = file.try_clone().map_err(|source| {
                    map_error(StageCreationError::Inspect {
                        path: stage.clone(),
                        source,
                    })
                })?;
                return Ok((
                    StageGuard::new(stage.clone(), Some(stage), metadata, source),
                    file,
                ));
            }
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(map_error(StageCreationError::Create {
                    path: stage,
                    source,
                }));
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    return Err(map_error(StageCreationError::NamespaceExhausted));
}

pub(crate) struct StageGuard {
    display_path: PathBuf,
    named_path: Option<PathBuf>,
    identity: fs::Metadata,
    source: File,
}

impl StageGuard {
    fn new(
        display_path: PathBuf,
        named_path: Option<PathBuf>,
        identity: fs::Metadata,
        source: File,
    ) -> Self {
        Self {
            display_path,
            named_path,
            identity,
            source,
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.display_path
    }

    pub(crate) fn verify(&self) -> io::Result<()> {
        let opened = self.source.metadata()?;
        if !opened.file_type().is_file() || !same_file_identity(&self.identity, &opened) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "publication stage descriptor identity changed",
            ));
        }
        if let Some(path) = self.named_path.as_deref() {
            let named = fs::symlink_metadata(path)?;
            if !named.file_type().is_file() || !same_file_identity(&opened, &named) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "publication stage name identity changed",
                ));
            }
        }
        Ok(())
    }

    /// Stops guarding the private stage while deliberately retaining its name.
    ///
    /// Portable filesystems do not offer an identity-conditional unlink. A
    /// verify-then-remove sequence could therefore unlink a caller replacement
    /// installed in the final window. Retaining bounded private debris is the
    /// conservative no-replacement contract.
    pub(crate) fn retain_private_stage(&mut self) {
        self.named_path = None;
    }

    pub(crate) fn has_named_stage(&self) -> bool {
        self.named_path.is_some()
    }

    pub(crate) fn source_metadata(&self) -> io::Result<fs::Metadata> {
        self.source.metadata()
    }

    pub(crate) fn publish_no_replace(&self, target: &Path) -> io::Result<()> {
        platform_publish_no_replace(&self.source, target)
    }
}

impl Drop for StageGuard {
    fn drop(&mut self) {}
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn publication_target(target: &Path) -> io::Result<(&Path, &std::ffi::OsStr)> {
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = target.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "publication target has no name",
        )
    })?;
    Ok((parent, name))
}

#[cfg(target_os = "macos")]
fn platform_publish_no_replace(source: &File, target: &Path) -> io::Result<()> {
    use rustix::fs::{CloneFlags, fclonefileat};

    let (parent, name) = publication_target(target)?;
    let directory = File::open(parent)?;
    fclonefileat(source, &directory, name, CloneFlags::empty()).map_err(Into::into)
}

#[cfg(target_os = "linux")]
fn platform_publish_no_replace(source: &File, target: &Path) -> io::Result<()> {
    use rustix::fs::{AtFlags, linkat};
    use std::os::fd::AsRawFd as _;

    let (parent, name) = publication_target(target)?;
    let directory = File::open(parent)?;
    let descriptor_path = format!("/proc/self/fd/{}", source.as_raw_fd());
    linkat(
        rustix::fs::CWD,
        descriptor_path,
        &directory,
        name,
        AtFlags::SYMLINK_FOLLOW,
    )
    .map_err(Into::into)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform_publish_no_replace(_source: &File, _target: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "descriptor-bound atomic no-replace publication is unavailable on this platform",
    ))
}

pub(crate) struct DirectoryWitness {
    path: PathBuf,
    identity: fs::Metadata,
}

#[derive(Debug)]
pub(crate) enum DirectoryWitnessError {
    Changed(&'static str),
    Io(io::Error),
}

impl DirectoryWitnessError {
    fn into_io(self) -> io::Error {
        match self {
            Self::Changed(reason) => io::Error::new(io::ErrorKind::InvalidData, reason),
            Self::Io(source) => source,
        }
    }
}

impl DirectoryWitness {
    pub(crate) fn capture(path: &Path) -> io::Result<Self> {
        let identity = fs::symlink_metadata(path)?;
        if !identity.file_type().is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "publication parent is not a non-symlink directory",
            ));
        }
        Ok(Self {
            path: path.to_path_buf(),
            identity,
        })
    }

    pub(crate) fn verify(&self) -> io::Result<()> {
        self.verify_detailed()
            .map_err(DirectoryWitnessError::into_io)
    }

    pub(crate) fn verify_detailed(&self) -> Result<(), DirectoryWitnessError> {
        let current = fs::symlink_metadata(&self.path).map_err(DirectoryWitnessError::Io)?;
        if !current.file_type().is_dir() {
            return Err(DirectoryWitnessError::Changed(
                "publication parent changed type",
            ));
        }
        #[cfg(any(unix, windows))]
        if !same_file_identity(&self.identity, &current) {
            return Err(DirectoryWitnessError::Changed(
                "publication parent directory identity changed",
            ));
        }
        Ok(())
    }
}

#[cfg(unix)]
pub(crate) fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(windows)]
pub(crate) fn same_file_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;

    left.volume_serial_number().is_some()
        && left.volume_serial_number() == right.volume_serial_number()
        && left.file_index().is_some()
        && left.file_index() == right.file_index()
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn same_file_identity(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    false
}

pub(crate) fn same_file_state(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.len() == right.len()
        && matches!(
            (left.modified(), right.modified()),
            (Ok(left_modified), Ok(right_modified)) if left_modified == right_modified
        )
}

pub(crate) fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(all(test, not(target_os = "linux")))]
mod tests {
    use super::*;

    #[test]
    fn named_private_stage_namespace_and_permissions_are_bounded() {
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random).unwrap();
        let directory = std::env::temp_dir().join(format!(
            "punctra-publication-stage-bound-{}-{}",
            std::process::id(),
            u64::from_le_bytes(random)
        ));
        fs::create_dir(&directory).unwrap();
        for _ in 0..MAX_NAMED_STAGES_PER_NAMESPACE {
            let result = create_stage(
                &directory,
                "bounded",
                || Ok::<(), StageCreationError>(()),
                |error| error,
            );
            let (stage, file) = result.expect("bounded stage slot must be available");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                assert_eq!(file.metadata().unwrap().permissions().mode() & 0o077, 0);
            }
            drop(file);
            drop(stage);
        }
        assert!(matches!(
            create_stage(
                &directory,
                "bounded",
                || Ok::<(), StageCreationError>(()),
                |error| error,
            ),
            Err(StageCreationError::NamespaceExhausted)
        ));
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 64);
        fs::remove_dir_all(directory).unwrap();
    }
}
