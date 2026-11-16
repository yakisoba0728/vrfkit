//! File-backed chunk source for the Reader. The decoder is shared with slices.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::mpsc::sync_channel;
use std::thread;

use vrf_container::{
    ChunkType, ContainerError, Preamble, PreambleReadError, decompress_replay_data_with_trailing,
    read_preamble,
};

use crate::error::CliError;
use crate::pass::{Chunk, Replay};

pub(crate) struct FileInput {
    main: FileChunks,
    ahead: FileChunks,
}

struct FileChunks {
    file: File,
    len: u64,
    position: u64,
}

impl FileInput {
    pub fn open(path: &Path) -> Result<(Preamble, Self), CliError> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        let preamble = read_preamble(&mut file).map_err(|error| match error {
            PreambleReadError::Io(error) => CliError::Io(error),
            PreambleReadError::Container(error) => CliError::Container(error),
        })?;
        let position = preamble.remaining_offset as u64;
        // try_clone shares the OS file position on some platforms. A second
        // read-only handle gives the prefetcher an independent cursor; the
        // documented input contract excludes concurrent file replacement.
        let mut ahead = File::open(path)?;
        if ahead.metadata()?.len() != len {
            return Err(std::io::Error::other("replay length changed while opening").into());
        }
        ahead.seek(SeekFrom::Start(position))?;
        Ok((
            preamble,
            Self {
                main: FileChunks {
                    file,
                    len,
                    position,
                },
                ahead: FileChunks {
                    file: ahead,
                    len,
                    position,
                },
            },
        ))
    }

    pub fn for_each_chunk(
        self,
        replay: &Replay<'_>,
        on_chunk: &mut dyn FnMut(Chunk<'_>) -> Result<(), CliError>,
    ) -> Result<u64, CliError> {
        let (compressed, encrypted) = (replay.compressed, replay.encrypted);
        let Self {
            mut main,
            mut ahead,
        } = self;
        thread::scope(|scope| {
            // Like the slice path, both cursors scan small chunk headers, but
            // each payload is read by one cursor only. A helper serving Event
            // chunks on this rendezvous would stop looking ahead at every
            // event, serializing decompression with the packet decode pass.
            let (tx, rx) = sync_channel(0);
            scope.spawn(move || {
                let mut encoded = Vec::new();
                loop {
                    let next = (|| {
                        while let Some((kind, size)) = ahead.next_header()? {
                            if kind == ChunkType::ReplayData {
                                encoded.resize(size, 0);
                                ahead.file.read_exact(&mut encoded)?;
                                return Ok(Some(decompress_replay_data_with_trailing(
                                    &encoded, compressed, encrypted,
                                )?));
                            }
                            ahead.skip_payload()?;
                        }
                        Ok::<_, CliError>(None)
                    })();
                    match next {
                        Ok(Some(plain)) => {
                            if tx.send(Ok(plain)).is_err() {
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(error) => {
                            let _ = tx.send(Err(error));
                            break;
                        }
                    }
                }
            });
            let mut unknown = 0;
            let mut event_payload = Vec::new();
            while let Some((kind, size)) = main.next_header()? {
                if kind == ChunkType::Event {
                    event_payload.resize(size, 0);
                    main.file.read_exact(&mut event_payload)?;
                    on_chunk(Chunk::Event(&event_payload))?;
                    continue;
                }
                main.skip_payload()?;
                match kind {
                    ChunkType::ReplayData => {
                        let (plain, unread) = rx.recv().map_err(|_| {
                            CliError::Usage("the decompression thread stopped".to_owned())
                        })??;
                        on_chunk(Chunk::ReplayData(plain, unread))?;
                    }
                    // The high-level Reader deliberately does not decode
                    // snapshots. Its callback only counts their occurrence.
                    ChunkType::Checkpoint => on_chunk(Chunk::Checkpoint(&[]))?,
                    ChunkType::Unknown(_) => unknown += 1,
                    ChunkType::Header => {}
                    ChunkType::Event => unreachable!(),
                }
            }
            Ok(unknown)
        })
    }
}

impl FileChunks {
    /// Validate a header against the complete file length before reading or
    /// seeking over its payload. `position` then names the next chunk header.
    fn next_header(&mut self) -> Result<Option<(ChunkType, usize)>, CliError> {
        let available = self.len.saturating_sub(self.position);
        if available == 0 {
            if self.file.metadata()?.len() != self.len {
                return Err(std::io::Error::other("replay length changed while reading").into());
            }
            return Ok(None);
        }
        if available < 8 {
            return Err(ContainerError::Truncated {
                context: "chunk header",
                needed: 8,
                available: available as usize,
            }
            .into());
        }
        let mut header = [0; 8];
        self.file.read_exact(&mut header)?;
        let kind = ChunkType::from_raw(u32::from_le_bytes(header[..4].try_into().unwrap()));
        let size = i32::from_le_bytes(header[4..].try_into().unwrap());
        if size < 0 {
            return Err(ContainerError::InvalidChunkSize { size }.into());
        }
        let available = available - 8;
        if size as u64 > available {
            return Err(ContainerError::Truncated {
                context: "chunk payload",
                needed: size as usize,
                available: available as usize,
            }
            .into());
        }
        self.position += 8 + size as u64;
        Ok(Some((kind, size as usize)))
    }

    fn skip_payload(&mut self) -> Result<(), CliError> {
        self.file.seek(SeekFrom::Start(self.position))?;
        Ok(())
    }
}
