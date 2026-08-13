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
                let cleanup_file = file.try_clone().map_err(|source| {
                    map_error(StageCreationError::Inspect {
                        path: stage.clone(),
                        source,
                    })
                })?;
                return Ok((
                    StageGuard::new(stage, parent.to_path_buf(), metadata, cleanup_file),
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
    file: File,
    linked: bool,
}

impl StageGuard {
    pub(crate) fn new(path: PathBuf, parent: PathBuf, identity: fs::Metadata, file: File) -> Self {
        Self {
            path: Some(path),
            parent,
            identity,
            file,
            linked: false,
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

    pub(crate) const fn mark_linked(&mut self) {
        self.linked = true;
    }

    pub(crate) fn remove(&mut self) -> io::Result<()> {
        if self.path.is_none() {
            return Ok(());
        }
        self.verify()?;
        if !self.linked {
            self.file.set_len(0)?;
            self.file.sync_all()?;
        }
        // There is no portable conditional-unlink operation. Retain the unique
        // alias. A published alias shares the target inode; an unpublished
        // alias has had its payload cleared through this already-owned handle.
        self.path = None;
        Ok(())
    }

    fn discard(&mut self) {
        if self.path.is_some() && self.remove().is_ok() {
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

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    #[test]
    fn cleanup_retains_only_cleared_or_published_stage_aliases() {
        let directory = TestDirectory::new();

        let (mut unpublished, mut unpublished_file) = stage(&directory.path);
        unpublished_file.write_all(b"unpublished payload").unwrap();
        unpublished_file.sync_all().unwrap();
        let unpublished_path = unpublished.path().to_path_buf();
        unpublished.remove().unwrap();
        assert_eq!(fs::metadata(unpublished_path).unwrap().len(), 0);

        let (mut published, mut published_file) = stage(&directory.path);
        published_file.write_all(b"published payload").unwrap();
        published_file.sync_all().unwrap();
        let published_path = published.path().to_path_buf();
        let target = directory.path.join("target.json");
        fs::hard_link(&published_path, &target).unwrap();
        published.mark_linked();
        published.remove().unwrap();
        assert!(same_file_identity(
            &fs::metadata(published_path).unwrap(),
            &fs::metadata(&target).unwrap()
        ));
        assert_eq!(fs::read(target).unwrap(), b"published payload");
    }

    fn stage(parent: &Path) -> (StageGuard, File) {
        create_stage(
            parent,
            "cleanup-test",
            || Ok::<(), io::Error>(()),
            map_stage_error,
        )
        .unwrap()
    }

    fn map_stage_error(error: StageCreationError) -> io::Error {
        match error {
            StageCreationError::RandomnessUnavailable => {
                io::Error::other("test stage randomness unavailable")
            }
            StageCreationError::NamespaceExhausted => {
                io::Error::other("test stage namespace exhausted")
            }
            StageCreationError::Inspect { path, source }
            | StageCreationError::Create { path, source } => {
                io::Error::new(source.kind(), format!("{}: {source}", path.display()))
            }
        }
    }

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new() -> Self {
            let mut random = [0; 16];
            getrandom::fill(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "punctra-publication-cleanup-{}-{}",
                std::process::id(),
                Hex(&random)
            ));
            fs::create_dir(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
