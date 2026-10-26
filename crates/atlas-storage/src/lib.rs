//! Append-only WAL with a committer thread (group commit).
//!
//! File layout: header `b"ATLASWAL"` + u32 LE version, then records
//! `[len u32][seq u64][crc32(payload) u32][payload]`, all little-endian.
//!
//! Durability: `append` returns Ok only after `sync_all` succeeded for the batch
//! containing the record. Covers process crash and OS-level flush via sync_all.
//! The parent directory is fsynced when the WAL/meta file is created on Unix; on
//! non-Unix targets (Windows) this is a documented no-op. See docs/DURABILITY.md.
//! Power-loss safety is NOT claimed.
//!
//! Apply order: records are applied (via the `Applier`) by the committer thread in
//! seq order, after the flush succeeded and before any ack is sent.
//!
//! On flush failure every commit in the batch gets Err and the file is truncated
//! back to the pre-batch length, so failed records are absent after reopen. If that
//! rollback (truncate) fails, the WAL best-effort zeroes the batch's first record
//! header and enters the Failed state: every later append returns `WAL_FAILED`
//! until the database is reopened. If the zeroing also fails, the outcome of the
//! failed batch is indeterminate after reopen (Err means "unknown").
use atlas_core::{AtlasError, Config, Durability};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
#[cfg(any(test, feature = "test-support"))]
use std::sync::{Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const MAGIC: &[u8; 8] = b"ATLASWAL";
/// WAL format version. v1 (16-byte unprotected record header) is refused, no migration.
const VERSION: u32 = 2;
const META_VERSION: u32 = 1;
const MARKER_VERSION: u32 = 1;
const HEADER_LEN: usize = 12;
/// v2 header: len u32 | !len u32 | seq u64 | flags u32 | payload_crc u32 | header_crc u32
const REC_HDR: usize = 28;
const HDR_CRC_COVERS: usize = 24;
const META_FILE: &str = "meta.bin";
const META_MAGIC: &[u8; 8] = b"ATLASMTA";
const META_LEN: usize = 24; // magic 8 + version 4 + dim 8 + crc32(first 20) 4

/// Called by the committer for each durable record, in seq order.
pub type Applier = Box<dyn FnMut(u64, &[u8]) + Send>;
pub type Replayed = Vec<(u64, Vec<u8>)>;
type Reply = Sender<Result<u64, AtlasError>>;

struct Req {
    payload: Vec<u8>,
    reply: Reply,
}

/// Test-only (feature `test-support`) deterministic synchronisation for the committer: a
/// "hold before write" latch and a counter of requests handed to it, so a test controls batch
/// composition without wall-clock assumptions. The latch is armed for ONE commit: the committer
/// blocks at the start of that commit (batch already assembled, nothing written) until
/// `release`. Every wait here is a deadlock safeguard, not a timing assumption.
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct CommitHold {
    armed: AtomicBool,
    st: Mutex<HoldState>,
    cv: Condvar,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct HoldState {
    held: bool,
    released: bool,
    enqueued: usize,
}

/// A hold that is never released frees itself after this long (deadlock safeguard).
#[cfg(any(test, feature = "test-support"))]
const HOLD_SAFEGUARD: Duration = Duration::from_secs(60);

#[cfg(any(test, feature = "test-support"))]
impl CommitHold {
    fn state(&self) -> std::sync::MutexGuard<'_, HoldState> {
        self.st.lock().unwrap_or_else(|p| p.into_inner())
    }
    /// The next commit blocks (before writing) until `release`.
    pub fn arm(&self) {
        let mut g = self.state();
        (g.held, g.released) = (false, false);
        drop(g);
        self.armed.store(true, Ordering::SeqCst);
    }
    /// Wait until the committer is blocked at the hold. False on timeout (a hang, not a result).
    pub fn wait_held(&self, limit: Duration) -> bool {
        let g = self.state();
        let (g, _) = self
            .cv
            .wait_timeout_while(g, limit, |s| !s.held)
            .unwrap_or_else(|p| p.into_inner());
        g.held
    }
    pub fn release(&self) {
        self.state().released = true;
        self.cv.notify_all();
    }
    /// Requests handed to the committer so far (counted right after the send).
    pub fn enqueued(&self) -> usize {
        self.state().enqueued
    }
    /// Wait until at least `n` requests were handed over. False on timeout.
    pub fn wait_enqueued(&self, n: usize, limit: Duration) -> bool {
        let g = self.state();
        let (g, _) = self
            .cv
            .wait_timeout_while(g, limit, |s| s.enqueued < n)
            .unwrap_or_else(|p| p.into_inner());
        g.enqueued >= n
    }
    fn note_enqueued(&self) {
        self.state().enqueued += 1;
        self.cv.notify_all();
    }
    fn hold_point(&self) {
        if !self.armed.swap(false, Ordering::SeqCst) {
            return;
        }
        let mut g = self.state();
        g.held = true;
        self.cv.notify_all();
        let _ = self
            .cv
            .wait_timeout_while(g, HOLD_SAFEGUARD, |s| !s.released);
    }
}

/// Fault-injection state. The real one exists only with `test-support` (or in unit tests);
/// production builds get a zero-sized stub whose `take_*` are constant `false`, so no hook
/// state, handle or setter is reachable from a release artifact.
#[cfg(any(test, feature = "test-support"))]
#[derive(Default, Clone)]
struct Faults {
    fail_flush: Arc<AtomicBool>,
    fail_truncate: Arc<AtomicBool>,
    fail_zero: Arc<AtomicBool>,
    partial_write: Arc<AtomicBool>,
    fail_marker: Arc<AtomicBool>,
    die_after_marker: Arc<AtomicBool>,
    flush_delay_ms: Arc<AtomicU64>,
    hold: Arc<CommitHold>,
}

#[cfg(any(test, feature = "test-support"))]
impl Faults {
    fn hold_point(&self) {
        self.hold.hold_point();
    }
    fn take_fail_flush(&self) -> bool {
        self.fail_flush.swap(false, Ordering::SeqCst)
    }
    fn take_fail_truncate(&self) -> bool {
        self.fail_truncate.swap(false, Ordering::SeqCst)
    }
    fn take_fail_zero(&self) -> bool {
        self.fail_zero.swap(false, Ordering::SeqCst)
    }
    fn take_partial_write(&self) -> bool {
        self.partial_write.swap(false, Ordering::SeqCst)
    }
    fn take_fail_marker(&self) -> bool {
        self.fail_marker.swap(false, Ordering::SeqCst)
    }
    fn take_die_after_marker(&self) -> bool {
        self.die_after_marker.swap(false, Ordering::SeqCst)
    }
    fn flush_delay_ms(&self) -> u64 {
        self.flush_delay_ms.load(Ordering::SeqCst)
    }
}

#[cfg(not(any(test, feature = "test-support")))]
#[derive(Default, Clone)]
struct Faults;

#[cfg(not(any(test, feature = "test-support")))]
impl Faults {
    fn hold_point(&self) {}
    fn take_fail_flush(&self) -> bool {
        false
    }
    fn take_fail_truncate(&self) -> bool {
        false
    }
    fn take_fail_zero(&self) -> bool {
        false
    }
    fn take_partial_write(&self) -> bool {
        false
    }
    fn take_fail_marker(&self) -> bool {
        false
    }
    fn take_die_after_marker(&self) -> bool {
        false
    }
    fn flush_delay_ms(&self) -> u64 {
        0
    }
}

pub struct Wal {
    tx: Option<Sender<Req>>,
    handle: Option<JoinHandle<()>>,
    #[cfg(any(test, feature = "test-support"))]
    faults: Faults,
    failed: Arc<AtomicBool>,
    max_record_len: u32,
}

/// fsync a directory so a newly created entry is durable. Unix only; elsewhere a
/// documented no-op (Windows cannot fsync a directory through std).
pub fn sync_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

fn header() -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[..8].copy_from_slice(MAGIC);
    h[8..].copy_from_slice(&VERSION.to_le_bytes());
    h
}

enum Parsed<'a> {
    Valid {
        seq: u64,
        end: usize,
        payload: &'a [u8],
    },
    /// REC_HDR zero bytes: explicit invalidation marker, ends the log.
    Marker,
    /// Header cut short, or a valid header whose payload is cut short (torn write).
    Incomplete,
    /// Complete header whose checksum / complement / reserved flags are invalid.
    BadHeader,
    /// Valid header, but the length exceeds the configured maximum.
    TooLarge(u32),
    /// Complete record with an invalid payload checksum.
    Invalid,
}

/// The header checksum is verified BEFORE the length is trusted.
fn parse(rest: &[u8], max_len: u32) -> Parsed<'_> {
    if rest.len() < REC_HDR {
        return Parsed::Incomplete;
    }
    let h = &rest[..REC_HDR];
    if h.iter().all(|b| *b == 0) {
        return Parsed::Marker;
    }
    let u32_at = |o: usize| u32::from_le_bytes(h[o..o + 4].try_into().expect("4 bytes"));
    if crc32fast::hash(&h[..HDR_CRC_COVERS]) != u32_at(24) {
        return Parsed::BadHeader;
    }
    let (len, comp, flags, payload_crc) = (u32_at(0), u32_at(4), u32_at(16), u32_at(20));
    if comp != !len || flags != 0 {
        return Parsed::BadHeader;
    }
    if len > max_len {
        return Parsed::TooLarge(len);
    }
    if rest.len() - REC_HDR < len as usize {
        return Parsed::Incomplete;
    }
    let payload = &rest[REC_HDR..REC_HDR + len as usize];
    if crc32fast::hash(payload) != payload_crc {
        return Parsed::Invalid;
    }
    Parsed::Valid {
        seq: u64::from_le_bytes(h[8..16].try_into().expect("8 bytes")),
        end: REC_HDR + len as usize,
        payload,
    }
}

/// Is there any valid record with seq > `last_seq` at or after `from`?
/// ponytail: byte-wise resync is O(n) per candidate offset; only runs on the error path.
fn valid_record_follows(bytes: &[u8], from: usize, last_seq: u64, max_len: u32) -> bool {
    (from..bytes.len())
        .any(|o| matches!(parse(&bytes[o..], max_len), Parsed::Valid { seq, .. } if seq > last_seq))
}

struct Scan {
    records: Replayed,
    good_len: usize,
    /// Set (repair mode only) when a COMPLETE final record with an invalid checksum was cut.
    invalid_seq: Option<u64>,
}

/// Policy (see docs/WAL_FORMAT.md):
/// - incomplete final header/payload: torn tail, stop at the last valid boundary;
/// - complete final record that is invalid: RECOVERY_REQUIRED (it may be an acked record that
///   rotted), unless `repair`, which cuts it and reports it so the caller keeps the bytes;
/// - damage with a valid record after it, or a valid record with an unexpected seq: WAL_CORRUPT.
fn scan(bytes: &[u8], max_len: u32, repair: bool) -> Result<Scan, AtlasError> {
    let mut off = HEADER_LEN;
    let mut records = Vec::new();
    let mut expect = 1u64;
    let mut invalid_seq = None;
    while off < bytes.len() {
        let kind = parse(&bytes[off..], max_len);
        match kind {
            Parsed::Valid { seq, end, payload } => {
                if seq != expect {
                    return Err(AtlasError::WalCorrupt(format!(
                        "non-monotonic seq at offset {off}: expected {expect}, found {seq}"
                    )));
                }
                records.push((seq, payload.to_vec()));
                off += end;
                expect += 1;
            }
            Parsed::Marker => break,
            Parsed::TooLarge(len) => {
                return Err(AtlasError::WalCorrupt(format!(
                    "record length {len} at offset {off} exceeds configured max {max_len}"
                )))
            }
            Parsed::Incomplete | Parsed::BadHeader | Parsed::Invalid => {
                if valid_record_follows(bytes, off + 1, expect - 1, max_len) {
                    return Err(AtlasError::WalCorrupt(format!(
                        "corrupt record at offset {off} with valid records after it (mid-log corruption)"
                    )));
                }
                if !matches!(kind, Parsed::Incomplete) {
                    if !repair {
                        return Err(AtlasError::RecoveryRequired(format!(
                            "complete final record (seq {expect}) at offset {off} has an invalid \
                             header or payload checksum; it was NOT discarded. Reopen with \
                             RecoverMode::Repair to cut it (removed bytes are kept in \
                             wal.corrupt-{expect}.bak)"
                        )));
                    }
                    invalid_seq = Some(expect);
                }
                break;
            }
        }
    }
    Ok(Scan {
        records,
        good_len: off,
        invalid_seq,
    })
}

impl Wal {
    /// Open (creating if needed), replay, truncate any torn/corrupt tail.
    /// `applier` is invoked by the committer for every record appended afterwards.
    pub fn open(
        path: &Path,
        cfg: &Config,
        applier: Applier,
    ) -> Result<(Wal, Replayed), AtlasError> {
        Self::open_with_failed_flag(path, cfg, applier, Arc::new(AtomicBool::new(false)))
    }

    /// Like `open`, with a caller-owned Failed flag. Setting it (e.g. from the `applier` when an
    /// applied record is rejected downstream) puts the WAL in the Failed state: the committer
    /// rejects every request that has not been written yet with `WAL_FAILED` and stops.
    pub fn open_with_failed_flag(
        path: &Path,
        cfg: &Config,
        applier: Applier,
        failed: Arc<AtomicBool>,
    ) -> Result<(Wal, Replayed), AtlasError> {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        handle_recovery_marker(path, dir, cfg.recover_mode)?;
        let existed = path.exists();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let h = header();
        let scanned = if bytes.len() < HEADER_LEN {
            if !h.starts_with(&bytes) {
                return Err(AtlasError::WalCorrupt("bad header".into()));
            }
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
            file.write_all(&h)?;
            Scan {
                records: Vec::new(),
                good_len: HEADER_LEN,
                invalid_seq: None,
            }
        } else {
            if bytes[..8] != MAGIC[..] {
                return Err(AtlasError::WalCorrupt("bad magic".into()));
            }
            let found = u32::from_le_bytes(bytes[8..HEADER_LEN].try_into().expect("4 bytes"));
            if found != VERSION {
                return Err(AtlasError::UnsupportedWalVersion {
                    found,
                    supported: VERSION,
                });
            }
            scan(
                &bytes,
                cfg.max_record_len,
                cfg.recover_mode == atlas_core::RecoverMode::Repair,
            )?
        };
        let Scan {
            records,
            good_len,
            invalid_seq,
        } = scanned;
        if let Some(seq) = invalid_seq {
            keep_evidence(dir, seq, &bytes[good_len..])?;
        }
        if good_len < bytes.len() {
            file.set_len(good_len as u64)?;
        }
        file.sync_all()?;
        if !existed {
            sync_dir(dir)?;
        }
        file.seek(SeekFrom::Start(good_len as u64))?;

        let next_seq = records.last().map_or(1, |(s, _)| s + 1);
        let max_batch = match cfg.durability {
            Durability::SyncPerCommit => 1,
            Durability::GroupCommit => cfg.max_batch.max(1),
        };
        let faults = Faults::default();
        let (tx, rx) = channel();
        let committer = Committer {
            file,
            len: good_len as u64,
            next_seq,
            max_batch,
            window: cfg.batch_window,
            faults: faults.clone(),
            dir: dir.to_path_buf(),
            failed: failed.clone(),
            applier,
        };
        let handle = std::thread::spawn(move || committer.run(rx));
        let wal = Wal {
            tx: Some(tx),
            handle: Some(handle),
            #[cfg(any(test, feature = "test-support"))]
            faults,
            failed,
            max_record_len: cfg.max_record_len,
        };
        Ok((wal, records))
    }

    /// Test hook: the next flush fails once (after the write, before it counts as durable).
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn fail_next_flush_handle(&self) -> Arc<AtomicBool> {
        self.faults.fail_flush.clone()
    }

    /// Test hook: the next rollback truncate fails once.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn fail_next_truncate_handle(&self) -> Arc<AtomicBool> {
        self.faults.fail_truncate.clone()
    }

    /// Test hook: the next best-effort header zeroing fails once.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn fail_next_zero_handle(&self) -> Arc<AtomicBool> {
        self.faults.fail_zero.clone()
    }

    /// Test hook: the next batch write is interrupted after half its bytes.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn partial_write_handle(&self) -> Arc<AtomicBool> {
        self.faults.partial_write.clone()
    }

    /// Test hook: the next recovery-marker write fails once.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn fail_next_marker_handle(&self) -> Arc<AtomicBool> {
        self.faults.fail_marker.clone()
    }

    /// Test hook: simulate process death right after the marker is written during rollback.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn die_after_marker_handle(&self) -> Arc<AtomicBool> {
        self.faults.die_after_marker.clone()
    }

    /// Test hook: milliseconds to sleep before every `sync_all` (widens the pending window).
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn flush_delay_handle(&self) -> Arc<AtomicU64> {
        self.faults.flush_delay_ms.clone()
    }

    /// Test hook: the committer hold latch and queued-request counter (see `CommitHold`).
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn commit_hold_handle(&self) -> Arc<CommitHold> {
        self.faults.hold.clone()
    }

    /// Blocks until the record is durable and applied; returns its seq.
    pub fn append(&self, payload: Vec<u8>) -> Result<u64, AtlasError> {
        if payload.len() > self.max_record_len as usize {
            return Err(AtlasError::RecordTooLarge {
                len: payload.len(),
                max: self.max_record_len,
            });
        }
        if self.failed.load(Ordering::SeqCst) {
            return Err(AtlasError::WalFailed);
        }
        let closed = || {
            if self.failed.load(Ordering::SeqCst) {
                AtlasError::WalFailed
            } else {
                AtlasError::WalClosed
            }
        };
        let (reply, rx) = channel();
        self.tx
            .as_ref()
            .ok_or(AtlasError::WalClosed)?
            .send(Req { payload, reply })
            .map_err(|_| closed())?;
        #[cfg(any(test, feature = "test-support"))]
        self.faults.hold.note_enqueued();
        rx.recv().map_err(|_| closed())?
    }
}

impl Drop for Wal {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

enum Rollback {
    /// Truncated back: the batch is provably absent and the WAL stays usable.
    Restored,
    /// Header zeroed: provably absent after restart, but this instance is poisoned.
    ProvenAbsent,
    /// Could not establish the on-disk state.
    Unknown,
}

struct Committer {
    file: File,
    len: u64,
    next_seq: u64,
    max_batch: usize,
    window: Duration,
    faults: Faults,
    dir: std::path::PathBuf,
    failed: Arc<AtomicBool>,
    applier: Applier,
}

impl Committer {
    fn run(mut self, rx: Receiver<Req>) {
        while let Ok(first) = rx.recv() {
            let mut batch = vec![first];
            let deadline = Instant::now() + self.window;
            while batch.len() < self.max_batch {
                let left = deadline.saturating_duration_since(Instant::now());
                match rx.recv_timeout(left) {
                    Ok(r) => batch.push(r),
                    Err(_) => break,
                }
            }
            if !self.commit(batch) {
                return; // Failed state: refuse all further writes
            }
        }
    }

    /// Restore the pre-batch file state.
    ///
    /// Design: the recovery marker is persisted FIRST, before any destructive step, so a
    /// crash or failure at any later point cannot leave a replayable failed batch unflagged.
    /// It is cleared again only once the WAL is provably valid (truncated, or header zeroed).
    /// `Unknown` means the on-disk state of the failed batch could not be established: the
    /// writer gets OUTCOME_UNKNOWN, which does NOT promise the batch is absent after restart.
    fn rollback(&mut self) -> Rollback {
        let marker_ok =
            !self.faults.take_fail_marker() && write_marker(&self.dir, self.len).is_ok();
        if marker_ok && self.faults.take_die_after_marker() {
            return Rollback::Unknown; // test hook: simulated process death after the marker
        }
        let truncated = !self.faults.take_fail_truncate()
            && self
                .file
                .set_len(self.len)
                .and_then(|_| self.file.seek(SeekFrom::Start(self.len)))
                .is_ok();
        if truncated {
            if marker_ok {
                let _ = clear_marker(&self.dir);
            }
            return Rollback::Restored;
        }
        let zeroed = !self.faults.take_fail_zero()
            && self
                .file
                .seek(SeekFrom::Start(self.len))
                .and_then(|_| self.file.write_all(&[0u8; REC_HDR]))
                .and_then(|_| self.file.sync_all())
                .is_ok();
        if zeroed {
            if marker_ok {
                let _ = clear_marker(&self.dir);
            }
            return Rollback::ProvenAbsent;
        }
        Rollback::Unknown
    }

    /// Returns false if the WAL entered the Failed state.
    fn commit(&mut self, batch: Vec<Req>) -> bool {
        self.faults.hold_point();
        // Failed (possibly set by the applier after the previous batch): write nothing new.
        if self.failed.load(Ordering::SeqCst) {
            for r in batch {
                let _ = r.reply.send(Err(AtlasError::WalFailed));
            }
            return false;
        }
        let mut buf = Vec::new();
        for (i, r) in batch.iter().enumerate() {
            let seq = self.next_seq + i as u64;
            let len = r.payload.len() as u32;
            let mut h = Vec::with_capacity(REC_HDR);
            h.extend_from_slice(&len.to_le_bytes());
            h.extend_from_slice(&(!len).to_le_bytes());
            h.extend_from_slice(&seq.to_le_bytes());
            h.extend_from_slice(&0u32.to_le_bytes()); // flags (reserved)
            h.extend_from_slice(&crc32fast::hash(&r.payload).to_le_bytes());
            let hcrc = crc32fast::hash(&h);
            h.extend_from_slice(&hcrc.to_le_bytes());
            buf.extend_from_slice(&h);
            buf.extend_from_slice(&r.payload);
        }
        let write = if self.faults.take_partial_write() {
            self.file
                .write_all(&buf[..buf.len() / 2])
                .and_then(|_| Err(std::io::Error::other("injected partial write")))
        } else {
            self.file.write_all(&buf)
        };
        let res = write.and_then(|_| {
            if self.faults.take_fail_flush() {
                return Err(std::io::Error::other("injected flush failure"));
            }
            let delay = self.faults.flush_delay_ms();
            if delay > 0 {
                std::thread::sleep(Duration::from_millis(delay));
            }
            self.file.sync_all()
        });
        match res {
            Ok(()) => {
                self.len += buf.len() as u64;
                let first = self.next_seq;
                self.next_seq += batch.len() as u64;
                for (i, r) in batch.iter().enumerate() {
                    (self.applier)(first + i as u64, &r.payload);
                }
                for (i, r) in batch.into_iter().enumerate() {
                    let _ = r.reply.send(Ok(first + i as u64));
                }
                // The applier may have set Failed (poison): stop now, in a controlled way. Dropping
                // the receiver wakes every queued request (they see Failed) and nothing more is
                // written; `Wal::drop` joins this thread.
                !self.failed.load(Ordering::SeqCst)
            }
            Err(e) => {
                let outcome = self.rollback();
                let unknown = matches!(outcome, Rollback::Unknown);
                let keep_going = matches!(outcome, Rollback::Restored);
                if !keep_going {
                    self.failed.store(true, Ordering::SeqCst); // poisoned until reopen
                }
                for r in batch {
                    let err = if unknown {
                        AtlasError::OutcomeUnknown(format!(
                            "write/sync failed ({e}) and the on-disk state could not be \
                             established; the operation may or may not exist after restart"
                        ))
                    } else {
                        AtlasError::WalFlushFailed(e.to_string())
                    };
                    let _ = r.reply.send(Err(err));
                }
                keep_going
            }
        }
    }
}

/// Read the persisted dimension; None if no meta file exists.
pub fn read_meta(dir: &Path) -> Result<Option<usize>, AtlasError> {
    let b = match std::fs::read(dir.join(META_FILE)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    if b.len() != META_LEN || &b[..8] != META_MAGIC {
        return Err(AtlasError::MetaCorrupt("bad length or magic".into()));
    }
    if u32::from_le_bytes(b[8..12].try_into().expect("4 bytes")) != META_VERSION {
        return Err(AtlasError::MetaCorrupt("unsupported version".into()));
    }
    if crc32fast::hash(&b[..20]) != u32::from_le_bytes(b[20..24].try_into().expect("4 bytes")) {
        return Err(AtlasError::MetaCorrupt("checksum mismatch".into()));
    }
    let dim = u64::from_le_bytes(b[12..20].try_into().expect("8 bytes"));
    usize::try_from(dim)
        .map(Some)
        .map_err(|_| AtlasError::MetaCorrupt("dimension out of range".into()))
}

/// Atomically create the meta file: write tmp, sync_all, rename, fsync directory.
pub fn write_meta(dir: &Path, dim: usize) -> Result<(), AtlasError> {
    let mut b = Vec::with_capacity(META_LEN);
    b.extend_from_slice(META_MAGIC);
    b.extend_from_slice(&META_VERSION.to_le_bytes());
    b.extend_from_slice(&(dim as u64).to_le_bytes());
    b.extend_from_slice(&crc32fast::hash(&b).to_le_bytes());
    let tmp = dir.join("meta.tmp");
    let mut f = File::create(&tmp)?;
    f.write_all(&b)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, dir.join(META_FILE))?;
    sync_dir(dir)?;
    Ok(())
}

const MARKER_FILE: &str = "recovery.required";
const MARKER_MAGIC: &[u8; 8] = b"ATLASREC";

/// Persist "the WAL is not provably valid; discard everything at/after `safe_len`".
fn write_marker(dir: &Path, safe_len: u64) -> Result<(), AtlasError> {
    let mut b = Vec::with_capacity(24);
    b.extend_from_slice(MARKER_MAGIC);
    b.extend_from_slice(&MARKER_VERSION.to_le_bytes());
    b.extend_from_slice(&safe_len.to_le_bytes());
    b.extend_from_slice(&crc32fast::hash(&b).to_le_bytes());
    let tmp = dir.join("recovery.tmp");
    let mut f = File::create(&tmp)?;
    f.write_all(&b)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, dir.join(MARKER_FILE))?;
    sync_dir(dir)?;
    Ok(())
}

fn read_marker(path: &Path) -> Result<u64, AtlasError> {
    let b = std::fs::read(path)?;
    let ok = b.len() == 24
        && &b[..8] == MARKER_MAGIC
        && u32::from_le_bytes(b[8..12].try_into().expect("4 bytes")) == MARKER_VERSION
        && crc32fast::hash(&b[..20]) == u32::from_le_bytes(b[20..24].try_into().expect("4 bytes"));
    if !ok {
        return Err(AtlasError::WalCorrupt(
            "recovery marker is damaged; manual repair required".into(),
        ));
    }
    Ok(u64::from_le_bytes(b[12..20].try_into().expect("8 bytes")))
}

/// Normal: refuse to open while a recovery marker exists. Repair: truncate the WAL to the
/// marker's safe length (dropping the never-acknowledged failed batch), then clear it.
fn handle_recovery_marker(
    wal_path: &Path,
    dir: &Path,
    mode: atlas_core::RecoverMode,
) -> Result<(), AtlasError> {
    let marker = dir.join(MARKER_FILE);
    if !marker.exists() {
        return Ok(());
    }
    if mode != atlas_core::RecoverMode::Repair {
        return Err(AtlasError::RecoveryRequired(
            "a previous write failure left the WAL unprovable; reopen with RecoverMode::Repair \
             to discard the failed (unacknowledged) batch"
                .into(),
        ));
    }
    let safe_len = read_marker(&marker)?;
    if wal_path.exists() {
        let f = OpenOptions::new().write(true).open(wal_path)?;
        if f.metadata()?.len() > safe_len {
            f.set_len(safe_len)?;
        }
        f.sync_all()?;
    }
    std::fs::remove_file(&marker)?;
    sync_dir(dir)?;
    Ok(())
}

fn clear_marker(dir: &Path) -> Result<(), AtlasError> {
    std::fs::remove_file(dir.join(MARKER_FILE))?;
    sync_dir(dir)?;
    Ok(())
}

/// Save the bytes about to be cut by a repair, durably, before touching the WAL.
fn keep_evidence(dir: &Path, seq: u64, removed: &[u8]) -> Result<(), AtlasError> {
    let mut name = format!("wal.corrupt-{seq}.bak");
    let mut n = 1;
    while dir.join(&name).exists() {
        name = format!("wal.corrupt-{seq}.{n}.bak");
        n += 1;
    }
    let mut f = File::create(dir.join(name))?;
    f.write_all(removed)?;
    f.sync_all()?;
    sync_dir(dir)?;
    Ok(())
}
