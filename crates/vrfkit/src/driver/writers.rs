//! Parquet writers running off the packet loop.
//!
//! `fields` and `movement` are the two large tables and their Parquet encoding
//! (Arrow batch build + ZSTD) was measured at 570 ms and 450 ms of a 2.60 s
//! export -- 37% of the run, executed inline in the packet loop. Each table is
//! an independent file whose writer never reads replay state, so each is moved
//! to its own thread and fed record batches over a bounded channel. The writers
//! still see every record exactly once, in stream order, and the row-group flush
//! boundary still falls on the same cumulative row counts, so the bytes are
//! unchanged; only the thread they are produced on differs.
//!
//! The channels are bounded so a slow writer applies backpressure instead of
//! growing the in-flight batch queue without limit. `actors`, `net_guids` and
//! `events` stay inline: together they are under 1% of the write cost.
//!
//! No error is dropped on this path. A writer that fails returns its error and
//! drops its receiver, which turns the next `send` into an error the packet loop
//! propagates; the deferred error is then recovered at join. A writer thread that
//! panics is reported as an error rather than being mistaken for success.

use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread;

use vrf_export::ExportError;

use crate::error::CliError;

/// Rows accumulated in the packet loop before a batch is handed to a writer
/// thread. A replay yields ~530 k packets but only ~0.8 field rows and ~3.5
/// movement rows per packet, so sending one message per packet would cost more
/// in channel traffic than the encoding it hides. At this size `fields` sends
/// ~26 messages and `movement` ~112 over a whole replay.
const WRITER_BATCH_ROWS: usize = 16_384;

/// Batches allowed in flight per writer. Bounds peak memory: four field batches
/// is roughly 10 MB of records plus the raw-bit payloads they own. The two
/// name columns no longer contribute -- they are interned `Arc<str>` shared
/// with the sink, so a queued batch holds refcounts, not strings.
const WRITER_QUEUE_DEPTH: usize = 4;

/// A writer running on its own thread, plus the handle needed to collect its
/// result. `T` is the record type of the table it owns.
pub(super) struct WriterThread<T> {
    tx: Option<SyncSender<Vec<T>>>,
    handle: Option<thread::JoinHandle<Result<(), ExportError>>>,
    batch: Vec<T>,
    /// Table name, used only to name the failing table in an error message.
    table: &'static str,
}

impl<T: Send + 'static> WriterThread<T> {
    /// Spawn a writer thread driven by `run`, which consumes every batch in
    /// stream order and then finalises the file.
    pub(super) fn spawn<F>(table: &'static str, run: F) -> Self
    where
        F: FnOnce(std::sync::mpsc::Receiver<Vec<T>>) -> Result<(), ExportError> + Send + 'static,
    {
        let (tx, rx) = sync_channel::<Vec<T>>(WRITER_QUEUE_DEPTH);
        let handle = thread::spawn(move || run(rx));
        Self {
            tx: Some(tx),
            handle: Some(handle),
            batch: Vec::with_capacity(WRITER_BATCH_ROWS),
            table,
        }
    }

    /// Move `records` into the pending batch, shipping it once it is full.
    pub(super) fn append(&mut self, records: &mut Vec<T>) -> Result<(), CliError> {
        self.batch.append(records);
        if self.batch.len() >= WRITER_BATCH_ROWS {
            self.ship()?;
        }
        Ok(())
    }

    fn ship(&mut self) -> Result<(), CliError> {
        let full = std::mem::replace(&mut self.batch, Vec::with_capacity(WRITER_BATCH_ROWS));
        let tx = self
            .tx
            .as_ref()
            .ok_or_else(|| CliError::Usage(format!("{} writer already closed", self.table)))?;
        // A send failure means the writer thread returned early with an error.
        // Report it as a send failure only if joining cannot produce the real
        // cause; `finish` below re-reads the thread result either way.
        tx.send(full)
            .map_err(|_| CliError::Usage(format!("{} writer stopped early", self.table)))
    }

    /// Ship the trailing partial batch, close the channel and surface the
    /// writer's own result. Any panic in the writer becomes an error here --
    /// it must never be mistaken for a completed file.
    pub(super) fn finish(mut self) -> Result<(), CliError> {
        let send_result = if self.batch.is_empty() {
            Ok(())
        } else {
            self.ship()
        };
        // Dropping the sender is what ends the writer loop.
        self.tx = None;
        let table = self.table;
        match self.handle.take().expect("writer handle present").join() {
            Ok(Ok(())) => send_result,
            // The writer's own error is the real cause; it supersedes a send
            // failure caused by that same early return.
            Ok(Err(e)) => Err(CliError::Export(e)),
            Err(_) => Err(CliError::Usage(format!("{table} writer thread panicked"))),
        }
    }
}

impl<T> Drop for WriterThread<T> {
    fn drop(&mut self) {
        // Every early return closes the producer side and waits for the writer
        // to observe cancellation. Dropping JoinHandle without joining would
        // detach a thread still writing into a staging directory that the
        // caller is about to remove or publish.
        self.tx = None;
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier, mpsc};
    use std::time::Duration;

    #[test]
    fn dropping_a_writer_closes_and_joins_its_thread() {
        let gate = Arc::new(Barrier::new(2));
        let writer_gate = Arc::clone(&gate);
        let writer = WriterThread::<u8>::spawn("test", move |rx| {
            for _ in rx {}
            writer_gate.wait();
            Ok(())
        });
        let (drop_done, dropped) = mpsc::channel();
        let dropper = thread::spawn(move || {
            drop(writer);
            drop_done.send(()).unwrap();
        });

        assert!(
            dropped.recv_timeout(Duration::from_millis(50)).is_err(),
            "drop returned while its writer thread was still running"
        );
        gate.wait();
        dropped.recv_timeout(Duration::from_secs(1)).unwrap();
        dropper.join().unwrap();
    }

    /// A writer that fails must never be reported as a finished file.
    ///
    /// Moving the Parquet writers onto threads moved their errors off the
    /// `?` path, which is exactly the shape of a silent success. This drives the
    /// failure deliberately: the writer returns an error and drops its receiver,
    /// the producing side sees a send failure, and `finish` must surface the
    /// writer's own error rather than either the send failure or `Ok`.
    #[test]
    fn a_failed_writer_thread_is_reported_not_swallowed() {
        let mut writer = WriterThread::<u8>::spawn("test", |rx| {
            // Take one batch, then fail -- the shape of a Parquet codec error.
            let _ = rx.recv();
            Err(ExportError::Usage("writer failed".into()))
        });

        // Keep shipping until the broken channel is observed, or until enough
        // batches have gone in to guarantee it would have been.
        let mut saw_send_failure = false;
        for _ in 0..(WRITER_QUEUE_DEPTH + 4) {
            let mut batch = vec![0u8; WRITER_BATCH_ROWS];
            if writer.append(&mut batch).is_err() {
                saw_send_failure = true;
                break;
            }
        }

        let err = writer
            .finish()
            .expect_err("a failed writer must not report success");
        assert!(
            err.to_string().contains("writer failed"),
            "finish must surface the writer's own error, got: {err}"
        );
        // Not asserted as required: whether the producer noticed first is a
        // race. What must hold is that finish reports the failure either way.
        let _ = saw_send_failure;
    }

    /// A panicking writer thread must also be an error. `JoinHandle::join`
    /// returns `Err` on panic and it would be easy to discard.
    ///
    /// The panic message this prints on stderr during `cargo test` is expected.
    #[test]
    fn a_panicking_writer_thread_is_reported_not_swallowed() {
        let writer = WriterThread::<u8>::spawn("test", |_rx| panic!("writer died"));
        let err = writer
            .finish()
            .expect_err("a panicking writer must not report success");
        assert!(
            err.to_string().contains("panicked"),
            "finish must name the panic, got: {err}"
        );
    }
}
