//! Same-volume staging and publication of one complete export directory.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_OUTPUT_PATH: AtomicU64 = AtomicU64::new(0);

/// Owns a unique sibling staging directory until it is either published or
/// abandoned.
///
/// The destination is never opened for writing. A failed decode therefore
/// drops this guard and removes only the generated staging directory, leaving
/// the last complete destination untouched. Publication uses same-parent
/// renames so the operation never crosses volumes.
pub(super) struct OutputTransaction {
    destination: PathBuf,
    staging: PathBuf,
    published: bool,
}

impl OutputTransaction {
    /// Create an empty, uniquely named staging directory beside `destination`.
    pub(super) fn begin(destination: &Path) -> io::Result<Self> {
        let file_name = destination.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "export destination must be a named directory, not a filesystem root",
            )
        })?;
        if destination.exists() && !destination.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "export destination exists but is not a directory: {}",
                    destination.display()
                ),
            ));
        }

        let parent = usable_parent(destination);
        fs::create_dir_all(parent)?;
        let staging = create_unique_directory(parent, file_name, "staging")?;
        Ok(Self {
            destination: destination.to_path_buf(),
            staging,
            published: false,
        })
    }

    /// Directory into which every output file must be written and finalised.
    pub(super) fn path(&self) -> &Path {
        &self.staging
    }

    /// Publish the completed staging directory as one directory rename.
    ///
    /// When a prior destination exists it is first moved to a unique sibling.
    /// If the staging rename then fails, that prior complete directory is moved
    /// straight back before the error is returned. Only after the new directory
    /// is in place is the backup removed.
    pub(super) fn publish(mut self) -> io::Result<()> {
        let prior = if self.destination.exists() {
            let backup = unique_sibling(&self.destination, "previous")?;
            fs::rename(&self.destination, &backup)?;
            Some(backup)
        } else {
            None
        };

        if let Err(publish_error) = fs::rename(&self.staging, &self.destination) {
            if let Some(backup) = prior {
                if let Err(restore_error) = fs::rename(&backup, &self.destination) {
                    return Err(io::Error::other(format!(
                        "could not publish {}: {publish_error}; could not restore prior output from {}: {restore_error}",
                        self.destination.display(),
                        backup.display()
                    )));
                }
            }
            return Err(publish_error);
        }

        self.published = true;
        if let Some(backup) = prior {
            // Publication is already committed. A cleanup failure must not be
            // reported as a failed export (which would falsely imply the old
            // destination was still active), but it must not be silent either.
            if let Err(error) = remove_generated(&backup) {
                eprintln!(
                    "warning: export published, but prior-output backup {} could not be removed: {error}",
                    backup.display()
                );
            }
        }
        Ok(())
    }
}

impl Drop for OutputTransaction {
    fn drop(&mut self) {
        if !self.published {
            let _ = remove_generated(&self.staging);
        }
    }
}

fn usable_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn generated_name(destination_name: &std::ffi::OsStr, kind: &str, nonce: u64) -> OsString {
    let mut name = OsString::from(".");
    name.push(destination_name);
    name.push(format!(".vrfkit-{kind}-{}-{nonce}", std::process::id()));
    name
}

fn create_unique_directory(
    parent: &Path,
    destination_name: &std::ffi::OsStr,
    kind: &str,
) -> io::Result<PathBuf> {
    loop {
        let nonce = NEXT_OUTPUT_PATH.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(generated_name(destination_name, kind, nonce));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

fn unique_sibling(destination: &Path, kind: &str) -> io::Result<PathBuf> {
    let name = destination.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination has no file name")
    })?;
    let parent = usable_parent(destination);
    loop {
        let nonce = NEXT_OUTPUT_PATH.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(generated_name(name, kind, nonce));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
}

/// Remove only paths generated by this module. `remove_dir_all` does not
/// follow directory symlinks, and the exact generated path is never derived
/// from an untrusted glob or environment variable.
fn remove_generated(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::OutputTransaction;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "vrfkit-publish-test-{}-{}",
                std::process::id(),
                NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).expect("create isolated test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn staging_entries(parent: &Path) -> Vec<PathBuf> {
        fs::read_dir(parent)
            .expect("read test directory")
            .map(|entry| entry.expect("read directory entry").path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains(".vrfkit-staging-"))
            })
            .collect()
    }

    #[test]
    fn an_aborted_export_preserves_the_prior_output_and_cleans_staging() {
        let root = TestDir::new();
        let destination = root.path().join("export");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("manifest.json"), b"old complete").unwrap();

        {
            let transaction = OutputTransaction::begin(&destination).unwrap();
            fs::write(transaction.path().join("manifest.json"), b"new partial").unwrap();
            // Fault injection: returning before publication drops the guard.
        }

        assert_eq!(
            fs::read(destination.join("manifest.json")).unwrap(),
            b"old complete"
        );
        assert!(staging_entries(root.path()).is_empty());
    }

    #[test]
    fn a_publication_failure_restores_the_prior_complete_directory() {
        let root = TestDir::new();
        let destination = root.path().join("export");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("manifest.json"), b"old complete").unwrap();

        let transaction = OutputTransaction::begin(&destination).unwrap();
        fs::write(transaction.path().join("manifest.json"), b"new complete").unwrap();
        // Fault injection after staging: make the second rename fail, after
        // publication has already moved the old destination aside.
        fs::remove_dir_all(transaction.path()).unwrap();
        transaction
            .publish()
            .expect_err("missing staging must fail");

        assert_eq!(
            fs::read(destination.join("manifest.json")).unwrap(),
            b"old complete"
        );
        assert!(staging_entries(root.path()).is_empty());
    }

    #[test]
    fn a_successful_publication_replaces_the_directory_as_one_complete_set() {
        let root = TestDir::new();
        let destination = root.path().join("export");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("old-only.parquet"), b"old").unwrap();

        let transaction = OutputTransaction::begin(&destination).unwrap();
        fs::write(transaction.path().join("manifest.json"), b"new complete").unwrap();
        transaction.publish().unwrap();

        assert_eq!(
            fs::read(destination.join("manifest.json")).unwrap(),
            b"new complete"
        );
        assert!(!destination.join("old-only.parquet").exists());
        assert!(staging_entries(root.path()).is_empty());
    }
}
