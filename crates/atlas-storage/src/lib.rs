pub mod segment;

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
const VERSION: u32 = 2;
const VERSION_BASED: u32 = 3;
const HEADER_LEN_BASED: usize = 20;
const META_VERSION: u32 = 1;
const MARKER_VERSION: u32 = 1;
const HEADER_LEN: usize = 12;
const REC_HDR: usize = 28;
const HDR_CRC_COVERS: usize = 24;
const META_FILE: &str = "meta.bin";
const META_MAGIC: &[u8; 8] = b"ATLASMTA";
const META_LEN: usize = 24;

pub type Applier = Box<dyn FnMut(u64, &[u8]) + Send>;
pub type Replayed = Vec<(u64, Vec<u8>)>;
type Reply = Sender<Result<u64, AtlasError>>;

struct Req {
    payload: Vec<u8>,
    reply: Reply,
}

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

#[cfg(any(test, feature = "test-support"))]
const HOLD_SAFEGUARD: Duration = Duration::from_secs(60);

#[cfg(any(test, feature = "test-support"))]
impl CommitHold {
    fn state(&self) -> std::sync::MutexGuard<'_, HoldState> {
        self.st.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub fn arm(&self) {
        let mut g = self.state();
        (g.held, g.released) = (false, false);
        drop(g);
        self.armed.store(true, Ordering::SeqCst);
    }
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
    pub fn enqueued(&self) -> usize {
        self.state().enqueued
    }
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
    Marker,
    Incomplete,
    BadHeader,
    TooLarge(u32),
    Invalid,
}

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

fn valid_record_follows(bytes: &[u8], from: usize, last_seq: u64, max_len: u32) -> bool {
    (from..bytes.len())
        .any(|o| matches!(parse(&bytes[o..], max_len), Parsed::Valid { seq, .. } if seq > last_seq))
}

struct Scan {
    base_seq: u64,
    records: Replayed,
    good_len: usize,
    invalid_seq: Option<u64>,
}

fn scan(
    bytes: &[u8],
    hdr_len: usize,
    base_seq: u64,
    max_len: u32,
    repair: bool,
) -> Result<Scan, AtlasError> {
    let mut off = hdr_len;
    let mut records = Vec::new();
    let mut expect = base_seq + 1;
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
        base_seq,
        records,
        good_len: off,
        invalid_seq,
    })
}

impl Wal {
    pub fn open(
        path: &Path,
        cfg: &Config,
        applier: Applier,
    ) -> Result<(Wal, Replayed), AtlasError> {
        Self::open_with_failed_flag(path, cfg, applier, Arc::new(AtomicBool::new(false)))
    }

    pub fn open_with_failed_flag(
        path: &Path,
        cfg: &Config,
        applier: Applier,
        failed: Arc<AtomicBool>,
    ) -> Result<(Wal, Replayed), AtlasError> {
        Self::open_checked(path, cfg, applier, failed, None)
    }

    /// Opens the log. With `segment_seq = Some(s)` the log must pair with a
    /// segment ending at `s`: its base may not exceed `s`, its last record may
    /// not precede `s`, and any prefix up to `s` is cut AFTER marker handling
    /// and tail repair, so those always see the original byte layout.
    pub fn open_checked(
        path: &Path,
        cfg: &Config,
        applier: Applier,
        failed: Arc<AtomicBool>,
        segment_seq: Option<u64>,
    ) -> Result<(Wal, Replayed), AtlasError> {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        handle_recovery_marker(path, dir, cfg.recover_mode)?;
        let (mut file, mut scanned) = recover_file(path, dir, cfg)?;
        // A leftover cut temp is dead: wal.log is intact until the cut's rename.
        match std::fs::remove_file(path.with_extension("log.truncating")) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        if let Some(seg) = segment_seq {
            if scanned.base_seq > seg {
                return Err(AtlasError::WalCorrupt(format!(
                    "log starts after sequence {} but the newest segment ends at {seg}                      (segment missing?)",
                    scanned.base_seq
                )));
            }
            if seg > scanned.base_seq {
                let last = scanned.records.last().map_or(scanned.base_seq, |(s, _)| *s);
                if last < seg {
                    return Err(AtlasError::WalCorrupt(format!(
                        "log ends at sequence {last} but the segment ends at {seg}"
                    )));
                }
                drop(file);
                truncate_prefix(path, seg, cfg.max_record_len)?;
                (file, scanned) = recover_file(path, dir, cfg)?;
            }
        }
        let Scan {
            base_seq,
            records,
            good_len,
            ..
        } = scanned;
        let next_seq = records.last().map_or(base_seq + 1, |(s, _)| s + 1);
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

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn fail_next_flush_handle(&self) -> Arc<AtomicBool> {
        self.faults.fail_flush.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn fail_next_truncate_handle(&self) -> Arc<AtomicBool> {
        self.faults.fail_truncate.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn fail_next_zero_handle(&self) -> Arc<AtomicBool> {
        self.faults.fail_zero.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn partial_write_handle(&self) -> Arc<AtomicBool> {
        self.faults.partial_write.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn fail_next_marker_handle(&self) -> Arc<AtomicBool> {
        self.faults.fail_marker.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn die_after_marker_handle(&self) -> Arc<AtomicBool> {
        self.faults.die_after_marker.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn flush_delay_handle(&self) -> Arc<AtomicU64> {
        self.faults.flush_delay_ms.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn commit_hold_handle(&self) -> Arc<CommitHold> {
        self.faults.hold.clone()
    }

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
    Restored,
    ProvenAbsent,
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
                return;
            }
        }
    }

    fn rollback(&mut self) -> Rollback {
        let marker_ok =
            !self.faults.take_fail_marker() && write_marker(&self.dir, self.len).is_ok();
        if marker_ok && self.faults.take_die_after_marker() {
            return Rollback::Unknown;
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

    fn commit(&mut self, batch: Vec<Req>) -> bool {
        self.faults.hold_point();
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
            h.extend_from_slice(&0u32.to_le_bytes());
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
                !self.failed.load(Ordering::SeqCst)
            }
            Err(e) => {
                let outcome = self.rollback();
                let unknown = matches!(outcome, Rollback::Unknown);
                let keep_going = matches!(outcome, Rollback::Restored);
                if !keep_going {
                    self.failed.store(true, Ordering::SeqCst);
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

/// Reads the log, repairs/validates it per `cfg`, syncs, and positions at the end.
fn recover_file(path: &Path, dir: &Path, cfg: &Config) -> Result<(File, Scan), AtlasError> {
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
            base_seq: 0,
            records: Vec::new(),
            good_len: HEADER_LEN,
            invalid_seq: None,
        }
    } else {
        if bytes[..8] != MAGIC[..] {
            return Err(AtlasError::WalCorrupt("bad magic".into()));
        }
        let found = u32::from_le_bytes(bytes[8..HEADER_LEN].try_into().expect("4 bytes"));
        let (hdr_len, base_seq) = match found {
            VERSION => (HEADER_LEN, 0),
            VERSION_BASED => {
                if bytes.len() < HEADER_LEN_BASED {
                    return Err(AtlasError::WalCorrupt("truncated v3 header".into()));
                }
                let base = u64::from_le_bytes(bytes[12..20].try_into().expect("8 bytes"));
                (HEADER_LEN_BASED, base)
            }
            _ => {
                return Err(AtlasError::UnsupportedWalVersion {
                    found,
                    supported: VERSION_BASED,
                })
            }
        };
        scan(
            &bytes,
            hdr_len,
            base_seq,
            cfg.max_record_len,
            cfg.recover_mode == atlas_core::RecoverMode::Repair,
        )?
    };
    if let Some(seq) = scanned.invalid_seq {
        keep_evidence(dir, seq, &bytes[scanned.good_len..])?;
    }
    if scanned.good_len < bytes.len() {
        file.set_len(scanned.good_len as u64)?;
    }
    file.sync_all()?;
    if !existed {
        sync_dir(dir)?;
    }
    file.seek(SeekFrom::Start(scanned.good_len as u64))?;
    Ok((file, scanned))
}

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

pub struct PrefixReport {
    pub base_seq: u64,
    pub dropped: usize,
    pub kept: usize,
}

pub fn truncate_prefix(
    path: &Path,
    through_seq: u64,
    max_record_len: u32,
) -> Result<PrefixReport, AtlasError> {
    let bytes = std::fs::read(path)?;
    if bytes.len() < HEADER_LEN || bytes[..8] != MAGIC[..] {
        return Err(AtlasError::WalCorrupt("bad header".into()));
    }
    let found = u32::from_le_bytes(bytes[8..HEADER_LEN].try_into().expect("4 bytes"));
    let (hdr_len, base_seq) = match found {
        VERSION => (HEADER_LEN, 0),
        VERSION_BASED if bytes.len() >= HEADER_LEN_BASED => (
            HEADER_LEN_BASED,
            u64::from_le_bytes(bytes[12..20].try_into().expect("8 bytes")),
        ),
        VERSION_BASED => return Err(AtlasError::WalCorrupt("truncated v3 header".into())),
        _ => {
            return Err(AtlasError::UnsupportedWalVersion {
                found,
                supported: VERSION_BASED,
            })
        }
    };
    if through_seq <= base_seq {
        let kept = scan(&bytes, hdr_len, base_seq, max_record_len, false)?
            .records
            .len();
        return Ok(PrefixReport {
            base_seq,
            dropped: 0,
            kept,
        });
    }
    let scanned = scan(&bytes, hdr_len, base_seq, max_record_len, false)?;
    if scanned.good_len != bytes.len() {
        return Err(AtlasError::WalCorrupt(
            "log has an unreadable tail; recover it before truncating".into(),
        ));
    }
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION_BASED.to_le_bytes());
    out.extend_from_slice(&through_seq.to_le_bytes());
    let (mut dropped, mut kept) = (0, 0);
    let mut off = hdr_len;
    for (seq, payload) in &scanned.records {
        let end = off + REC_HDR + payload.len();
        if *seq > through_seq {
            out.extend_from_slice(&bytes[off..end]);
            kept += 1;
        } else {
            dropped += 1;
        }
        off = end;
    }
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = path.with_extension("log.truncating");
    let mut f = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)?;
    f.write_all(&out)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)?;
    sync_dir(dir)?;
    Ok(PrefixReport {
        base_seq: through_seq,
        dropped,
        kept,
    })
}
