use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

pub(crate) enum StageCreationError {
    RandomnessUnavailable,
    NamespaceExhausted,
    Inspect { path: PathBuf, source: io::Error },
    Create { path: PathBuf, source: io::Error },
}

pub(crate) fn create_stage<E>(
    parent: &Path,
    namespace: &'static str,
    mut before_attempt: impl FnMut() -> Result<(), E>,
    mut map_error: impl FnMut(StageCreationError) -> E,
) -> Result<(StageGuard, File), E> {
    for _ in 0..64 {
        before_attempt()?;
        let mut random = [0; 16];
        getrandom::fill(&mut random)
            .map_err(|_| map_error(StageCreationError::RandomnessUnavailable))?;
        let stage = parent.join(format!(".punctra-{namespace}-{}.tmp", Hex(&random)));
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&stage)
        {
            Ok(file) => {
                let metadata = file.metadata().map_err(|source| {
                    map_error(StageCreationError::Inspect {
                        path: stage.clone(),
                        source,
                    })
                })?;
                return Ok((StageGuard::new(stage, parent.to_path_buf(), metadata), file));
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
    Err(map_error(StageCreationError::NamespaceExhausted))
}

struct Hex<'a>(&'a [u8]);

impl fmt::Display for Hex<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

pub(crate) struct StageGuard {
    path: Option<PathBuf>,
    parent: PathBuf,
    identity: fs::Metadata,
}

impl StageGuard {
    pub(crate) fn new(path: PathBuf, parent: PathBuf, identity: fs::Metadata) -> Self {
        Self {
            path: Some(path),
            parent,
            identity,
        }
    }

    pub(crate) fn path(&self) -> &Path {
        self.path
            .as_deref()
            .expect("a live publication stage has a path")
    }

    pub(crate) fn verify(&self) -> io::Result<()> {
        let metadata = fs::symlink_metadata(self.path())?;
        if metadata.file_type().is_file() && same_file_identity(&self.identity, &metadata) {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "publication stage identity changed",
            ))
        }
    }

    pub(crate) fn remove(&mut self) -> io::Result<()> {
        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        self.verify()?;
        fs::remove_file(path)?;
        self.path = None;
        Ok(())
    }

    fn discard(&mut self) {
        if self.path.is_some() && self.verify().is_ok() && self.remove().is_ok() {
            let _ = sync_directory(&self.parent);
        }
    }
}

impl Drop for StageGuard {
    fn drop(&mut self) {
        self.discard();
    }
}

pub(crate) struct DirectoryWitness {
    path: PathBuf,
    identity: fs::Metadata,
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
        let current = fs::symlink_metadata(&self.path)?;
        if !current.file_type().is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "publication parent changed type",
            ));
        }
        #[cfg(any(unix, windows))]
        if !same_file_identity(&self.identity, &current) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
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

pub(crate) fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}
