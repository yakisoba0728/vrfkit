//! Unified error type for the CLI.

use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use thiserror::Error;
use vrf_net::pipeline::ReplicationReader;

/// Every failure the CLI and [`crate::ReplayReader`] report. Library users
/// see it as `vrfkit::ReplayError`; new causes are added as variants.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CliError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("container error: {0}")]
    Container(#[from] vrf_container::ContainerError),

    #[error("frame error: {0}")]
    Frame(#[from] vrf_frame::FrameError),

    #[error("checkpoint {checkpoint}: {error}")]
    CheckpointSchema {
        checkpoint: String,
        #[source]
        error: vrf_schema::SchemaError,
    },

    /// A container or frame failure inside one checkpoint snapshot, kept
    /// apart from the same failure in the main stream. `checkpoint_id` is
    /// absent when the chunk failed before its id was read.
    #[error(
        "checkpoint {}: {error}",
        checkpoint_label(*checkpoint_index, checkpoint_id.as_deref())
    )]
    Checkpoint {
        checkpoint_index: u32,
        checkpoint_id: Option<String>,
        #[source]
        error: Box<CliError>,
    },

    /// The replay's branch has no registered payload transform; reported
    /// before any record, never decoded with a guessed transform.
    #[error("unsupported branch: {message}")]
    UnsupportedBranch { branch: String, message: String },

    #[error("export error: {0}")]
    Export(#[from] vrf_export::ExportError),

    #[error("{0}")]
    Usage(String),
}

/// The wire id, or the zero-based chunk index when the id never parsed.
fn checkpoint_label(index: u32, id: Option<&str>) -> String {
    id.map_or_else(|| format!("#{index}"), str::to_owned)
}

impl CliError {
    /// Attach a snapshot's identity to a container or frame failure raised
    /// inside it. Other errors (consumer stops, export writes, schema errors
    /// that already name the checkpoint) pass through unchanged.
    pub(crate) fn in_checkpoint(self, checkpoint_index: u32, checkpoint_id: Option<&str>) -> Self {
        match self {
            Self::Container(_) | Self::Frame(_) => Self::Checkpoint {
                checkpoint_index,
                checkpoint_id: checkpoint_id.map(str::to_owned),
                error: Box::new(self),
            },
            other => other,
        }
    }
}

/// `error`, prefixed with the path it happened on. The kind is kept, and the
/// original error stays reachable through `Error::source`.
pub(crate) fn io_at(path: &Path, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        PathError {
            path: path.to_path_buf(),
            source: error,
        },
    )
}

/// Open a replay for reading, naming the path on failure. A directory is
/// refused by name: Windows reports it only as "access denied", and elsewhere
/// the open succeeds and the first read fails.
pub(crate) fn open_input(path: &Path) -> io::Result<File> {
    let is_directory = || {
        io::Error::new(
            io::ErrorKind::IsADirectory,
            format!("{}: is a directory, not a replay file", path.display()),
        )
    };
    match File::open(path) {
        Ok(file) => match file.metadata() {
            Ok(metadata) if metadata.is_dir() => Err(is_directory()),
            Ok(_) => Ok(file),
            Err(error) => Err(io_at(path, error)),
        },
        Err(_) if path.is_dir() => Err(is_directory()),
        Err(error) => Err(io_at(path, error)),
    }
}

/// [`open_input`], then the whole file.
pub(crate) fn read_input(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = open_input(path)?;
    // The length is a hint, as in `fs::read`; the read decides.
    let hint = file.metadata().map_or(0, |metadata| metadata.len());
    let mut data = Vec::with_capacity(usize::try_from(hint).unwrap_or(0));
    file.read_to_end(&mut data)
        .map_err(|error| io_at(path, error))?;
    Ok(data)
}

#[derive(Debug)]
struct PathError {
    path: PathBuf,
    source: io::Error,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.source)
    }
}

impl std::error::Error for PathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// A replication reader for `branch`, or the error every subcommand reports
/// for a branch it cannot read. A helper, not a `From` conversion, so no
/// other vrf-net error is ever labelled a branch error.
pub fn replication_reader(branch: &str) -> Result<ReplicationReader, CliError> {
    ReplicationReader::new(branch).map_err(|e| CliError::UnsupportedBranch {
        branch: branch.to_owned(),
        message: e.to_string(),
    })
}
