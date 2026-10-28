//! Embedded Atlas engine: insert -> WAL (durable ack) -> exact index.
use atlas_core::{
    clip_echo, clip_text, validate_vector, AtlasError, Config, EffectiveScope, Metric, PermSet,
    TenantId, VectorId,
};
use atlas_index::{AnnIndex, FlatIndex};
use atlas_storage::{read_meta, write_meta, Applier, Wal};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard};

pub use atlas_index::Hit;

/// One stored record as returned by `fetch` and streamed by `snapshot` / `export`.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: VectorId,
    pub vector: Vec<f32>,
    pub metadata: Value,
}

/// Receives streamed records. An `Err` aborts the stream.
pub type Sink<'a> = &'a mut dyn FnMut(Item) -> Result<(), AtlasError>;

type Factory = Box<dyn Fn(usize) -> BoxedIndex + Send + Sync>;

/// One index per tenant, created lazily. A key is always (tenant, id): there is no shared id space.
struct Parts {
    map: HashMap<TenantId, BoxedIndex>,
    make: Factory,
    dim: usize,
}

impl Parts {
    fn apply(&mut self, t: TenantId, r: Record) -> Result<(), AtlasError> {
        if r.deleted {
            if let Some(ix) = self.map.get_mut(&t) {
                ix.remove(r.id);
            }
            return Ok(());
        }
        let (dim, make) = (self.dim, &self.make);
        let ix = self.map.entry(t).or_insert_with(|| make(dim));
        ix.insert(r.id, &r.vector, r.metadata)
    }
}

#[derive(Serialize, Deserialize, PartialEq)]
struct Record {
    /// Owning tenant. Required: a record without a valid tenant is corruption, never defaulted.
    tenant: String,
    id: VectorId,
    #[serde(default)]
    vector: Vec<f32>,
    #[serde(default)]
    metadata: Value,
    /// Tombstone: removes `id`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    op_id: Option<String>,
}

/// The index is derived from the WAL and rebuilt on every open; any `AnnIndex` can be swapped in.
pub type BoxedIndex = Box<dyn AnnIndex + Send + Sync>;

const MAX_OP_ID_LEN: usize = 128;

/// First request seen for an (tenant, operation id): its seq, a cheap fingerprint, and the exact
/// canonical payload bytes (the encoded record: tenant, id, vector, metadata, op_id; no seq or
/// timestamp). Byte equality decides whether a retry is identical; the fingerprint is only a
/// pre-check.
struct OpEntry {
    seq: u64,
    fp: u64,
    payload: Arc<[u8]>,
}

/// Payload fingerprint function (replaceable only under `test-support`, to force collisions).
type FpFn = fn(&[u8]) -> u64;

/// (seq, fingerprint, payload) copied out of an `OpEntry`.
type OpLookup = (u64, u64, Arc<[u8]>);

/// Bounded by `Config.max_ops` for new writes (CAPACITY_EXCEEDED); replay always admits what is
/// already durable. Retains each op's payload bytes (up to max_record_len). ponytail: no expiry;
/// add expiry/compaction when snapshots exist.
type OpMap = HashMap<(TenantId, String), OpEntry>;

/// Documented cap on metadata nesting (arrays/objects). Below serde_json recursion limit of 128
/// (the record itself adds one level) so every admitted record replays.
pub const MAX_METADATA_DEPTH: usize = 100;

/// FNV-1a 64 over the WAL payload bytes. Live retries hash the bytes they are about to write
/// (admit proved these decode back to the same record); replay hashes the bytes read from the
/// WAL. Only a pre-check: equality of the stored bytes decides idempotency.
fn fingerprint(payload: &[u8]) -> u64 {
    payload.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100000001b3)
    })
}

/// Nesting depth of `v` (scalar = 1, each nested array/object adds 1), computed with an explicit stack: no recursion.
fn metadata_depth(v: &Value) -> usize {
    let (mut max, mut stack) = (0, vec![(v, 1usize)]);
    while let Some((v, d)) = stack.pop() {
        max = max.max(d);
        match v {
            Value::Array(a) => stack.extend(a.iter().map(|c| (c, d + 1))),
            Value::Object(m) => stack.extend(m.values().map(|c| (c, d + 1))),
            _ => {}
        }
    }
    max
}

/// Drop a value without recursion (the derived drop of `Value` is recursive).
fn drop_iteratively(v: Value) {
    let mut stack = vec![v];
    while let Some(v) = stack.pop() {
        match v {
            Value::Array(a) => stack.extend(a),
            Value::Object(m) => stack.extend(m.into_iter().map(|(_, c)| c)),
            _ => {}
        }
    }
}

/// Owns a caller's metadata from the first line of a write until it has passed the depth cap, so
/// EVERY early return (authz, poison, depth, ...) tears a deep value down iteratively.
struct Metadata(Option<Value>);

impl Drop for Metadata {
    fn drop(&mut self) {
        if let Some(v) = self.0.take() {
            drop_iteratively(v);
        }
    }
}

/// Reject over-deep metadata before any clone/serialize. Past this, depth <= MAX_METADATA_DEPTH
/// and the derived drop is safe.
fn check_depth(mut guard: Metadata) -> Result<Value, AtlasError> {
    let metadata = guard.0.take().unwrap_or(Value::Null);
    if metadata_depth(&metadata) <= MAX_METADATA_DEPTH {
        return Ok(metadata);
    }
    drop_iteratively(metadata);
    Err(AtlasError::InvalidMetadata(format!(
        "metadata nesting exceeds {MAX_METADATA_DEPTH} levels"
    )))
}

fn decode(seq: u64, payload: &[u8]) -> Result<(TenantId, Record), AtlasError> {
    let bad = |e: &dyn std::fmt::Display| AtlasError::WalCorrupt(format!("seq {seq}: {e}"));
    let r: Record = serde_json::from_slice(payload).map_err(|e| bad(&e))?;
    let t = TenantId::parse(&r.tenant).map_err(|e| bad(&e))?;
    Ok((t, r))
}

/// Lock a mutex that guards plain data with no invariant a panic could break (poison text, test
/// hooks, the op gate): a poisoned lock is recovered, never turned into a panic.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What every caller except the culprit sees once the engine is poisoned. Deliberately constant:
/// no index error text, no seq or tenant of anyone's record (detail: `set_diagnostics_sink`).
const POISON_MESSAGE: &str = "engine poisoned by an earlier internal failure; no further \
     operations are accepted until the database is reopened";

fn generic_poison() -> AtlasError {
    AtlasError::WalCorrupt(POISON_MESSAGE.into())
}

/// OUTCOME_UNKNOWN text for a record that was written before the poison became visible.
fn outcome_unknown(has_op_id: bool) -> AtlasError {
    AtlasError::OutcomeUnknown(
        if has_op_id {
            "outcome unknown: the record may be durable; reopen the engine and retry the same op \
             id (it returns the original seq if the record is durable and inserts it once \
             otherwise)"
        } else {
            "outcome unknown: the record may be durable; reopen the engine to learn the result \
             (plain insert/delete have no operation id and are not idempotently retryable)"
        }
        .into(),
    )
}

fn lock_error(what: &str) -> AtlasError {
    AtlasError::WalCorrupt(format!("{what} lock poisoned by an earlier panic"))
}

fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    match (p.downcast_ref::<&str>(), p.downcast_ref::<String>()) {
        (Some(s), _) => (*s).to_string(),
        (_, Some(s)) => s.clone(),
        _ => "non-string panic payload".into(),
    }
}

/// Drop a panic payload; a payload whose own drop panics must not unwind further.
fn drop_quietly(p: Box<dyn std::any::Any + Send>) {
    let _ = catch_unwind(AssertUnwindSafe(move || drop(p)));
}

fn panic_error(seq: u64, p: Box<dyn std::any::Any + Send>) -> AtlasError {
    let text = panic_text(&*p);
    drop_quietly(p);
    AtlasError::WalCorrupt(format!("seq {seq}: index panicked: {text}"))
}

/// Decode, note the op id and apply one WAL payload; MAY PANIC if the `AnnIndex` does. The live
/// applier calls this inside `catch_unwind`; everything else goes through `replay_one`.
fn replay_inner(
    parts: &RwLock<Parts>,
    ops: &Mutex<OpMap>,
    seq: u64,
    payload: &[u8],
    fp: FpFn,
) -> Result<(), AtlasError> {
    let (t, r) = decode(seq, payload)?;
    if let Some(op) = &r.op_id {
        ops.lock()
            .map_err(|_| lock_error("op map"))?
            .entry((t.clone(), op.clone()))
            .or_insert_with(|| OpEntry {
                seq,
                fp: fp(payload),
                payload: Arc::from(payload),
            });
    }
    parts
        .write()
        .map_err(|_| lock_error("index"))?
        .apply(t, r)
        .map_err(|e| AtlasError::WalCorrupt(format!("seq {seq}: {e}")))
}

/// `replay_inner` with an index panic turned into a typed error (used by open-time replay).
fn replay_one(
    parts: &RwLock<Parts>,
    ops: &Mutex<OpMap>,
    seq: u64,
    payload: &[u8],
    fp: FpFn,
) -> Result<(), AtlasError> {
    match catch_unwind(AssertUnwindSafe(|| {
        replay_inner(parts, ops, seq, payload, fp)
    })) {
        Ok(r) => r,
        Err(p) => Err(panic_error(seq, p)),
    }
}

/// Remember the first applier failure; later writes surface it instead of dropping it. Returns
/// the recorded detail iff THIS call recorded the first failure.
fn note_failure(poison: &Mutex<Option<String>>, res: Result<(), AtlasError>) -> Option<String> {
    let e = res.err()?;
    let mut g = lock(poison);
    if g.is_some() {
        return None;
    }
    let detail = clip_text(&e.to_string());
    *g = Some(detail.clone());
    Some(detail)
}

/// Embedder-registered operator channel for the full poison detail (see docs/SECURITY.md).
pub type DiagnosticsSink = Arc<dyn Fn(&str) + Send + Sync>;

/// Call the sink (if any) with no engine lock held; a panicking sink changes nothing.
fn call_sink(sink: &Mutex<Option<DiagnosticsSink>>, detail: &str) {
    let s = lock(sink).clone();
    if let Some(s) = s {
        if let Err(p) = catch_unwind(AssertUnwindSafe(|| s(detail))) {
            drop_quietly(p);
        }
    }
}

/// Which layer rejected a request because of poison (test-support counters).
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy)]
enum Layer {
    /// The cheap check before any payload work.
    Cheap = 0,
    /// The pre-check at the top of `append`, right before the WAL is touched.
    AppendPrecheck = 1,
    /// The WAL Failed flag (set by the applier; enforced by `Wal::append` / the committer).
    WalFailedFlag = 2,
}

/// Per-layer poison rejection counts (test-support).
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoisonLayers {
    pub cheap: usize,
    pub append_precheck: usize,
    pub wal_failed_flag: usize,
}

pub struct Engine {
    dim: usize,
    max_record_len: u32,
    max_ops: usize,
    max_tenant_vectors: usize,
    wal: Wal,
    parts: Arc<RwLock<Parts>>,
    ops: Arc<Mutex<OpMap>>,
    fp: FpFn,
    /// Full detail of the first applier failure (decode error, index error or index panic),
    /// bounded by `clip_text`. PRIVATE: only the culprit's error and the diagnostics sink carry it.
    poison: Arc<Mutex<Option<String>>>,
    /// Set (before the poison text) when an `AnnIndex` PANICKED in the applier: the index may be
    /// partially mutated, so every read fails closed too.
    panicked: Arc<AtomicBool>,
    sink: Arc<Mutex<Option<DiagnosticsSink>>>,
    /// Test-only: skip the engine's own poison checks (cheap + append pre-check).
    #[cfg(any(test, feature = "test-support"))]
    bypass_poison_checks: AtomicBool,
    #[cfg(any(test, feature = "test-support"))]
    layer_counts: [AtomicUsize; 3],
    /// Seq of the record whose apply failed first (0 = unknown). Lets `append` tell the culprit
    /// (WAL_CORRUPT) from an innocent record written in the same batch (OUTCOME_UNKNOWN).
    poison_seq: Arc<AtomicU64>,
    /// Test-only pause point (see `set_before_gate_hook`).
    #[cfg(any(test, feature = "test-support"))]
    before_gate: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Serializes op-id inserts so check-then-append is atomic (they do not group-commit together).
    op_gate: Mutex<()>,
}

fn resolve_dim(dir: &Path, requested: Option<usize>) -> Result<usize, AtlasError> {
    match (read_meta(dir)?, requested) {
        (Some(stored), Some(requested)) if stored != requested => {
            Err(AtlasError::DimensionMismatch { stored, requested })
        }
        (Some(stored), _) => Ok(stored),
        (None, _) if dir.join("wal.log").exists() => Err(AtlasError::InvalidConfig(
            "wal.log exists without meta.bin".into(),
        )),
        (None, Some(d)) if d > 0 => {
            write_meta(dir, d)?;
            Ok(d)
        }
        (None, _) => Err(AtlasError::InvalidConfig(
            "a positive dimension is required to create a database".into(),
        )),
    }
}

/// Permission check first (Forbidden), yielding the caller's tenant.
fn authz(scope: &EffectiveScope, perm: PermSet) -> Result<&TenantId, AtlasError> {
    match scope.tenant() {
        Some(t) if scope.allows(perm) => Ok(t),
        _ => Err(AtlasError::Forbidden),
    }
}

impl Engine {
    /// Create (`cfg.dim` required) or reopen (`cfg.dim` optional; if given it must match
    /// the persisted dimension, else DIMENSION_MISMATCH).
    pub fn open(dir: impl AsRef<Path>, cfg: Config) -> Result<Engine, AtlasError> {
        Self::open_with_index(dir, cfg, |dim| Box::new(FlatIndex::new(dim)))
    }

    /// Like `open`, with a caller-chosen empty-index factory, called once per tenant (lazily)
    /// with the resolved dimension. The WAL is replayed into it; persisted index files are
    /// never trusted. Any record without a valid tenant fails the open (WAL_CORRUPT).
    pub fn open_with_index(
        dir: impl AsRef<Path>,
        cfg: Config,
        make_index: impl Fn(usize) -> BoxedIndex + Send + Sync + 'static,
    ) -> Result<Engine, AtlasError> {
        Self::open_inner(dir.as_ref(), cfg, make_index, fingerprint)
    }

    /// Test-only (feature `test-support`): `open` with a replacement fingerprint function, used
    /// by live and replay alike, to force fingerprint collisions.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn open_with_fingerprint(
        dir: impl AsRef<Path>,
        cfg: Config,
        fp: fn(&[u8]) -> u64,
    ) -> Result<Engine, AtlasError> {
        Self::open_inner(dir.as_ref(), cfg, |dim| Box::new(FlatIndex::new(dim)), fp)
    }

    fn open_inner(
        dir: &Path,
        cfg: Config,
        make_index: impl Fn(usize) -> BoxedIndex + Send + Sync + 'static,
        fp: FpFn,
    ) -> Result<Engine, AtlasError> {
        std::fs::create_dir_all(dir)?;
        let dim = resolve_dim(dir, cfg.dim)?;
        let parts = Arc::new(RwLock::new(Parts {
            map: HashMap::new(),
            make: Box::new(make_index),
            dim,
        }));
        let ops: Arc<Mutex<OpMap>> = Arc::new(Mutex::new(HashMap::new()));
        let poison: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let poison_seq = Arc::new(AtomicU64::new(0));
        let wal_failed = Arc::new(AtomicBool::new(false));
        let panicked = Arc::new(AtomicBool::new(false));
        let sink: Arc<Mutex<Option<DiagnosticsSink>>> = Arc::new(Mutex::new(None));
        let (ap, ao, apo) = (parts.clone(), ops.clone(), poison.clone());
        let (aps, awf) = (poison_seq.clone(), wal_failed.clone());
        let (apanic, asink) = (panicked.clone(), sink.clone());
        // The whole per-record apply (decode, op-map note, index mutation) runs inside
        // catch_unwind: a panicking `AnnIndex` must never kill the committer (see SECURITY.md,
        // "Poison and panic policy"). Requires `panic = "unwind"`.
        let applier: Applier = Box::new(move |seq, payload| {
            if apanic.load(Ordering::SeqCst) {
                return; // the index may be partially mutated: apply nothing more (still durable)
            }
            let (res, did_panic) = match catch_unwind(AssertUnwindSafe(|| {
                replay_inner(&ap, &ao, seq, payload, fp)
            })) {
                Ok(r) => (r, false),
                Err(p) => (Err(panic_error(seq, p)), true),
            };
            if res.is_ok() {
                return;
            }
            if did_panic {
                apanic.store(true, Ordering::SeqCst); // reads fail closed from now on
            }
            let _ = aps.compare_exchange(0, seq, Ordering::SeqCst, Ordering::SeqCst);
            let first = note_failure(&apo, res);
            // Poison is recorded first; the committer then refuses to write anything queued.
            awf.store(true, Ordering::SeqCst);
            if let Some(detail) = first {
                call_sink(&asink, &detail);
            }
        });
        let (wal, records) =
            Wal::open_with_failed_flag(&dir.join("wal.log"), &cfg, applier, wal_failed)?;
        for (seq, payload) in records {
            replay_one(&parts, &ops, seq, &payload, fp)?;
        }
        Ok(Engine {
            dim,
            max_record_len: cfg.max_record_len,
            max_ops: cfg.max_ops,
            max_tenant_vectors: cfg.max_tenant_vectors,
            wal,
            parts,
            ops,
            fp,
            poison,
            panicked,
            sink,
            #[cfg(any(test, feature = "test-support"))]
            bypass_poison_checks: AtomicBool::new(false),
            #[cfg(any(test, feature = "test-support"))]
            layer_counts: Default::default(),
            poison_seq,
            #[cfg(any(test, feature = "test-support"))]
            before_gate: Mutex::new(None),
            op_gate: Mutex::new(()),
        })
    }

    /// Reject with the GENERIC poison error if the engine is poisoned. `layer` only matters to the
    /// test-support attribution counters and bypass flag.
    #[cfg(any(test, feature = "test-support"))]
    fn reject_if_poisoned(&self, layer: Layer) -> Result<(), AtlasError> {
        if self.bypass_poison_checks.load(Ordering::SeqCst) {
            return Ok(());
        }
        if lock(&self.poison).is_some() {
            self.layer_counts[layer as usize].fetch_add(1, Ordering::SeqCst);
            return Err(generic_poison());
        }
        Ok(())
    }

    #[cfg(not(any(test, feature = "test-support")))]
    fn reject_if_poisoned(&self) -> Result<(), AtlasError> {
        match &*lock(&self.poison) {
            Some(_) => Err(generic_poison()),
            None => Ok(()),
        }
    }

    /// The cheap check, before any payload work.
    fn check_poison(&self) -> Result<(), AtlasError> {
        #[cfg(any(test, feature = "test-support"))]
        return self.reject_if_poisoned(Layer::Cheap);
        #[cfg(not(any(test, feature = "test-support")))]
        self.reject_if_poisoned()
    }

    /// Fail-closed read access: after an index PANIC the in-memory state may be inconsistent and
    /// is never served; a poisoned lock (a panic while the write guard was held) is the same
    /// condition. Both give the generic poison error.
    fn read_parts(&self) -> Result<RwLockReadGuard<'_, Parts>, AtlasError> {
        if self.panicked.load(Ordering::SeqCst) {
            return Err(generic_poison());
        }
        self.parts.read().map_err(|_| generic_poison())
    }

    /// Admission check run BEFORE the WAL is touched: the exact bytes about to be written must be
    /// within the record limit, decode through `decode` (the function replay and the live
    /// applier use) and decode to a record EQUAL to `rec`; anything else is INVALID_METADATA.
    fn admit(&self, payload: &[u8], rec: &Record) -> Result<(), AtlasError> {
        if payload.len() > self.max_record_len as usize {
            return Err(AtlasError::RecordTooLarge {
                len: payload.len(),
                max: self.max_record_len,
            });
        }
        match decode(0, payload) {
            Ok((_, back)) if back == *rec => Ok(()),
            Ok(_) => Err(AtlasError::InvalidMetadata(
                "record does not round-trip through the WAL encoding".into(),
            )),
            Err(AtlasError::WalCorrupt(m)) => Err(AtlasError::InvalidMetadata(m)),
            Err(e) => Err(e),
        }
    }

    /// Encode + admit + fingerprint: everything expensive, done without any engine lock.
    fn prepare(&self, rec: &Record) -> Result<(Vec<u8>, u64), AtlasError> {
        let payload = serde_json::to_vec(rec).map_err(|e| AtlasError::Codec(e.to_string()))?;
        self.admit(&payload, rec)?;
        let fp = (self.fp)(&payload);
        Ok((payload, fp))
    }

    /// Test-only pause point, run after the cheap poison check and before the gate / append.
    #[cfg(any(test, feature = "test-support"))]
    fn pause_point(&self) {
        let hook = lock(&self.before_gate).clone();
        if let Some(h) = hook {
            h();
        }
    }

    #[cfg(not(any(test, feature = "test-support")))]
    #[inline(always)]
    fn pause_point(&self) {}

    /// Poison handling around the WAL append (under op_gate for insert_op). `has_op_id` only
    /// selects the OUTCOME_UNKNOWN wording.
    /// - already poisoned: reject before writing (generic error);
    /// - poison set while the request was queued: the committer refuses to write it and returns
    ///   WAL_FAILED (or drops it when it stops), reported as the generic poison error (the record
    ///   is provably absent);
    /// - poison set once the record was written: the record IS durable. Its own apply failure is
    ///   WAL_CORRUPT with the detail; any other record is OUTCOME_UNKNOWN with no detail.
    fn append(&self, payload: Vec<u8>, has_op_id: bool) -> Result<u64, AtlasError> {
        #[cfg(any(test, feature = "test-support"))]
        self.reject_if_poisoned(Layer::AppendPrecheck)?;
        #[cfg(not(any(test, feature = "test-support")))]
        self.reject_if_poisoned()?;
        let seq = match self.wal.append(payload) {
            Err(AtlasError::WalFailed | AtlasError::WalClosed) if lock(&self.poison).is_some() => {
                #[cfg(any(test, feature = "test-support"))]
                self.layer_counts[Layer::WalFailedFlag as usize].fetch_add(1, Ordering::SeqCst);
                return Err(generic_poison());
            }
            other => other?,
        };
        let poisoned = lock(&self.poison).clone();
        match poisoned {
            None => Ok(seq),
            Some(m) if self.poison_seq.load(Ordering::SeqCst) == seq => {
                Err(AtlasError::WalCorrupt(m))
            }
            Some(_) => Err(outcome_unknown(has_op_id)),
        }
    }

    /// Returns the WAL seq once the record reached the configured durability boundary
    /// and has been applied to the caller's tenant partition (in WAL order).
    pub fn insert(
        &self,
        scope: &EffectiveScope,
        id: VectorId,
        vector: &[f32],
        metadata: Value,
    ) -> Result<u64, AtlasError> {
        let metadata = Metadata(Some(metadata));
        let t = authz(scope, PermSet::WRITE)?;
        self.check_poison()?; // cheap: before any depth/encode/admit work
        let metadata = check_depth(metadata)?;
        validate_vector(self.dim, vector)?;
        self.check_tenant_capacity(t)?;
        let (payload, _) = self.prepare(&Record {
            tenant: t.as_str().to_string(),
            deleted: false,
            id,
            vector: vector.to_vec(),
            metadata,
            op_id: None,
        })?;
        self.pause_point();
        self.append(payload, false)
    }

    /// Idempotent insert keyed by (tenant, operation id) (1..=128 bytes).
    ///
    /// Retrying the same `op_id` with the same payload (after OUTCOME_UNKNOWN, a crash or a
    /// restart) never applies twice and returns the ORIGINAL seq. Same `op_id` with a different
    /// payload is OP_ID_CONFLICT. Op ids of different tenants are independent namespaces.
    /// The dedupe window is the whole WAL, bounded by `Config.max_ops` (see OpMap).
    /// OUTCOME_UNKNOWN means the record may be durable (e.g. the engine was poisoned by another
    /// record in the same batch): treat it as unknown and retry the same op id after reopen.
    pub fn insert_op(
        &self,
        scope: &EffectiveScope,
        op_id: &str,
        id: VectorId,
        vector: &[f32],
        metadata: Value,
    ) -> Result<u64, AtlasError> {
        let metadata = Metadata(Some(metadata));
        let t = authz(scope, PermSet::WRITE)?;
        self.check_poison()?; // cheap: before any depth/encode/admit work
        let metadata = check_depth(metadata)?;
        if op_id.is_empty() || op_id.len() > MAX_OP_ID_LEN {
            return Err(AtlasError::InvalidOpId);
        }
        validate_vector(self.dim, vector)?;
        // Encode/admit/fingerprint outside the gate; under it only the op-map check + append.
        let (payload, fp) = self.prepare(&Record {
            tenant: t.as_str().to_string(),
            deleted: false,
            id,
            vector: vector.to_vec(),
            metadata,
            op_id: Some(op_id.to_string()),
        })?;
        self.pause_point();
        let _gate = lock(&self.op_gate);
        // Decision under the gate: identical retry only if the stored bytes (this tenant's entry)
        // equal ours; the fingerprint merely short-circuits the byte comparison. The ops mutex is
        // held only to copy (seq, fp, Arc) out; the (up to 16 MiB) comparison runs without it.
        if let Some((seq, stored_fp, stored)) = self.lookup_op(t, op_id)? {
            let same = stored_fp == fp && *stored == payload[..];
            return if same {
                Ok(seq)
            } else {
                Err(AtlasError::OpIdConflict(clip_echo(op_id)))
            };
        }
        // New op id: capacity is checked only now so a retry of a known op still succeeds when
        // full. The gate serialises insert_op, so the op-map bound is exact.
        if self.ops.lock().map_err(|_| generic_poison())?.len() >= self.max_ops {
            return Err(AtlasError::CapacityExceeded("operation map"));
        }
        self.check_tenant_capacity(t)?;
        self.append(payload, true)
    }

    /// Refuse an upsert when the tenant partition is at `max_tenant_vectors` (updates of existing
    /// ids are refused too: conservative, no per-id lookup). ponytail: read-then-append is not
    /// atomic, concurrent writers can overshoot by the in-flight count; tighten if that matters.
    fn check_tenant_capacity(&self, t: &TenantId) -> Result<(), AtlasError> {
        let n = self.read_parts()?.map.get(t).map_or(0, |ix| ix.len());
        if n >= self.max_tenant_vectors {
            return Err(AtlasError::CapacityExceeded("tenant partition"));
        }
        Ok(())
    }

    /// Copy one op-map entry out and release the map lock before returning.
    fn lookup_op(&self, t: &TenantId, op_id: &str) -> Result<Option<OpLookup>, AtlasError> {
        let ops = self.ops.lock().map_err(|_| generic_poison())?;
        let found = ops.get(&(t.clone(), op_id.to_string()));
        Ok(found.map(|o| (o.seq, o.fp, Arc::clone(&o.payload))))
    }

    pub fn search(
        &self,
        scope: &EffectiveScope,
        q: &[f32],
        k: usize,
        metric: Metric,
    ) -> Result<Vec<Hit>, AtlasError> {
        let t = authz(scope, PermSet::SEARCH)?;
        validate_vector(self.dim, q)?;
        match self.read_parts()?.map.get(t) {
            Some(ix) => ix.search(q, k, metric),
            None => Ok(vec![]),
        }
    }

    /// Top-k among the caller's ids accepted by `allow`; the filter only narrows (exact for
    /// the default index).
    pub fn search_filtered(
        &self,
        scope: &EffectiveScope,
        q: &[f32],
        k: usize,
        metric: Metric,
        allow: &dyn Fn(VectorId) -> bool,
    ) -> Result<Vec<Hit>, AtlasError> {
        let t = authz(scope, PermSet::SEARCH)?;
        validate_vector(self.dim, q)?;
        match self.read_parts()?.map.get(t) {
            Some(ix) => ix.search_filtered(q, k, metric, allow),
            None => Ok(vec![]),
        }
    }

    pub fn fetch(&self, scope: &EffectiveScope, id: VectorId) -> Result<Item, AtlasError> {
        let t = authz(scope, PermSet::FETCH)?;
        self.items(t)?
            .into_iter()
            .find(|i| i.id == id)
            .ok_or(AtlasError::NotFound)
    }

    pub fn count(&self, scope: &EffectiveScope) -> Result<usize, AtlasError> {
        let t = authz(scope, PermSet::COUNT)?;
        let g = self.read_parts()?;
        Ok(g.map.get(t).map_or(0, |ix| ix.len()))
    }

    /// Ok(true) if a record was removed.
    pub fn delete(&self, scope: &EffectiveScope, id: VectorId) -> Result<bool, AtlasError> {
        let t = authz(scope, PermSet::DELETE)?;
        self.check_poison()?; // cheap: before any lookup/encode work
        if !self.items(t)?.iter().any(|i| i.id == id) {
            return Ok(false);
        }
        let (payload, _) = self.prepare(&Record {
            tenant: t.as_str().to_string(),
            id,
            vector: vec![],
            metadata: Value::Null,
            op_id: None,
            deleted: true,
        })?;
        self.pause_point();
        self.append(payload, false)?;
        Ok(true)
    }

    /// Streams the caller's records to `sink` after authorization. `Ok(n)` is the only
    /// complete result; any failure mid-stream is `Incomplete { emitted, reason }`. The view
    /// is a point-in-time copy of ONE tenant taken under a single read guard, released before
    /// the sink runs. A sink that panics unwinds through this call: no result is returned (so
    /// never `Ok`), no lock is held and no state changes (see docs/SECURITY.md).
    pub fn snapshot(&self, scope: &EffectiveScope, sink: Sink) -> Result<u64, AtlasError> {
        self.stream(authz(scope, PermSet::SNAPSHOT)?, sink)
    }

    /// Same contract as `snapshot`, under the EXPORT permission.
    pub fn export(&self, scope: &EffectiveScope, sink: Sink) -> Result<u64, AtlasError> {
        self.stream(authz(scope, PermSet::EXPORT)?, sink)
    }

    fn stream(&self, t: &TenantId, sink: Sink) -> Result<u64, AtlasError> {
        let mut n = 0;
        for item in self.items(t)? {
            sink(item).map_err(|e| AtlasError::Incomplete {
                emitted: n,
                reason: clip_text(&e.to_string()),
            })?;
            n += 1;
        }
        Ok(n)
    }

    /// Records of one tenant only (empty if it has no partition).
    fn items(&self, t: &TenantId) -> Result<Vec<Item>, AtlasError> {
        let g = self.read_parts()?;
        Ok(g.map.get(t).map_or_else(Vec::new, |ix| {
            ix.entries()
                .into_iter()
                .map(|(id, vector, metadata)| Item {
                    id,
                    vector,
                    metadata,
                })
                .collect()
        }))
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Register the operator diagnostics sink: called ONCE, at poison time, with the full (bounded)
    /// detail of the first applier failure (index error or index panic), on the committer thread,
    /// with no engine lock held. A panic inside the sink is caught and ignored. The sink must not
    /// block: the committer waits for it before acknowledging the batch. A sink registered after
    /// the engine is already poisoned is never called. It is an in-process trust boundary: the
    /// detail may contain index internals and must not be forwarded to tenants.
    pub fn set_diagnostics_sink(&self, sink: DiagnosticsSink) {
        *lock(&self.sink) = Some(sink);
    }

    /// Test-only: the private full poison detail.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn poison_detail(&self) -> Option<String> {
        lock(&self.poison).clone()
    }

    /// Test-only: how many requests each layer rejected because of poison.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn poison_layer_counts(&self) -> PoisonLayers {
        let n = |l: Layer| self.layer_counts[l as usize].load(Ordering::SeqCst);
        PoisonLayers {
            cheap: n(Layer::Cheap),
            append_precheck: n(Layer::AppendPrecheck),
            wal_failed_flag: n(Layer::WalFailedFlag),
        }
    }

    /// Test-only: when `on`, the engine's own poison checks (cheap and `append` pre-check) are
    /// skipped, so only the WAL Failed flag layer can reject a request.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn bypass_engine_poison_check(&self, on: bool) {
        self.bypass_poison_checks.store(on, Ordering::SeqCst);
    }

    /// Total records across ALL tenants. Test-only (feature `test-support`); tenant-visible
    /// counts go through `count`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn len(&self) -> usize {
        self.read_parts()
            .map_or(0, |g| g.map.values().map(|ix| ix.len()).sum())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Test-only (feature `test-support`): run `hook` at the pause point of every write, i.e.
    /// after the cheap poison check and immediately before `op_gate` (insert_op) or the append
    /// (insert, delete). The hook runs without any engine lock held and may block.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn set_before_gate_hook(&self, hook: impl Fn() + Send + Sync + 'static) {
        *lock(&self.before_gate) = Some(Arc::new(hook));
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn fail_next_flush_handle(&self) -> Arc<AtomicBool> {
        self.wal.fail_next_flush_handle()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn fail_next_truncate_handle(&self) -> Arc<AtomicBool> {
        self.wal.fail_next_truncate_handle()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn fail_next_zero_handle(&self) -> Arc<AtomicBool> {
        self.wal.fail_next_zero_handle()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn partial_write_handle(&self) -> Arc<AtomicBool> {
        self.wal.partial_write_handle()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn fail_next_marker_handle(&self) -> Arc<AtomicBool> {
        self.wal.fail_next_marker_handle()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn die_after_marker_handle(&self) -> Arc<AtomicBool> {
        self.wal.die_after_marker_handle()
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn flush_delay_handle(&self) -> Arc<AtomicU64> {
        self.wal.flush_delay_handle()
    }

    /// Test-only: committer hold latch + queued-request counter (deterministic batch control).
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn commit_hold_handle(&self) -> Arc<atlas_storage::CommitHold> {
        self.wal.commit_hold_handle()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::{AuthContext, KeyScope, Principal};
    use std::sync::atomic::AtomicUsize;

    fn scope(t: &str) -> EffectiveScope {
        let t = TenantId::parse(t).unwrap();
        AuthContext::for_test(
            Principal {
                tenant: t.clone(),
                perms: PermSet::ALL,
            },
            KeyScope {
                tenant: t,
                allowed: PermSet::ALL,
            },
        )
        .effective()
    }

    #[test]
    fn applier_failure_is_surfaced_not_dropped() {
        let dir = std::env::temp_dir().join(format!("atlas-poison-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let e = Engine::open(&dir, Config::new(2)).unwrap();
        let s = scope("a");
        e.insert(&s, VectorId(1), &[1.0, 0.0], Value::Null).unwrap();
        // A tenantless payload reaching the live applier must poison the instance.
        let res = replay_one(&e.parts, &e.ops, 99, br#"{"id":1}"#, e.fp);
        assert_eq!(res.as_ref().unwrap_err().code(), "WAL_CORRUPT");
        note_failure(&e.poison, res);
        let err = e.insert(&s, VectorId(2), &[1.0, 0.0], Value::Null);
        assert_eq!(err.unwrap_err().code(), "WAL_CORRUPT");
        // Reads of already-applied data still work; denied calls stay Forbidden first.
        assert_eq!(e.count(&s).unwrap(), 1);
        let none = EffectiveScope::default();
        let err = e.insert(&none, VectorId(3), &[1.0, 0.0], Value::Null);
        assert_eq!(err.unwrap_err().code(), "FORBIDDEN");
    }

    #[test]
    fn tombstone_for_unknown_tenant_is_a_noop() {
        let dir = std::env::temp_dir().join(format!("atlas-tomb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let e = Engine::open(&dir, Config::new(2)).unwrap();
        replay_one(
            &e.parts,
            &e.ops,
            1,
            br#"{"tenant":"zz","id":1,"deleted":true}"#,
            e.fp,
        )
        .unwrap();
        assert!(e.is_empty());
    }

    #[test]
    fn admit_rejects_a_record_that_does_not_round_trip_to_the_request() {
        let dir = std::env::temp_dir().join(format!("atlas-admit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let e = Engine::open(&dir, Config::new(2)).unwrap();
        let rec = |id| Record {
            tenant: "a".into(),
            id: VectorId(id),
            vector: vec![1.0, 0.0],
            metadata: Value::Null,
            deleted: false,
            op_id: None,
        };
        let payload = serde_json::to_vec(&rec(1)).unwrap();
        e.admit(&payload, &rec(1)).unwrap();
        let err = e.admit(&payload, &rec(2)).unwrap_err();
        assert_eq!(err.code(), "INVALID_METADATA");
    }

    #[test]
    fn poison_text_is_bounded() {
        let poison = Mutex::new(None);
        note_failure(&poison, Err(AtlasError::Codec("x".repeat(1 << 20))));
        let m = poison.lock().unwrap().clone().unwrap();
        assert!(m.chars().count() <= 300, "{}", m.len());
    }

    #[test]
    fn non_round_trippable_record_is_rejected_before_the_wal() {
        let dir = std::env::temp_dir().join(format!("atlas-nrt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let e = Engine::open(&dir, Config::new(2)).unwrap();
        let s = scope("a");
        let seq0 = e.insert(&s, VectorId(1), &[1.0, 0.0], Value::Null).unwrap();
        let len0 = std::fs::metadata(dir.join("wal.log")).unwrap().len();
        // NaN serializes as null and cannot decode back to f32: not a round trip.
        let bad = Record {
            tenant: "a".into(),
            id: VectorId(2),
            vector: vec![f32::NAN, 0.0],
            metadata: Value::Null,
            deleted: false,
            op_id: Some("op".into()),
        };
        let err = e.prepare(&bad).unwrap_err();
        assert_eq!(err.code(), "INVALID_METADATA");
        assert_eq!(std::fs::metadata(dir.join("wal.log")).unwrap().len(), len0);
        assert!(e.ops.lock().unwrap().is_empty(), "no op-map entry");
        assert_eq!(e.count(&s).unwrap(), 1);
        let next = e.insert(&s, VectorId(3), &[1.0, 0.0], Value::Null).unwrap();
        assert_eq!(next, seq0 + 1, "no seq consumed");
    }

    fn deep(levels: usize) -> Value {
        (1..levels).fold(Value::Null, |v, _| Value::Array(vec![v]))
    }

    #[test]
    fn poison_error_takes_precedence_over_payload_errors() {
        let dir = std::env::temp_dir().join(format!("atlas-prec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let e = Engine::open(&dir, Config::new(2)).unwrap();
        let s = scope("a");
        note_failure(&e.poison, Err(AtlasError::Codec("boom".into())));
        let too_deep = || deep(MAX_METADATA_DEPTH + 1);
        let r = e.insert(&s, VectorId(1), &[1.0, 0.0], too_deep());
        assert_eq!(r.unwrap_err().code(), "WAL_CORRUPT");
        let r = e.insert_op(&s, "op", VectorId(1), &[1.0, 0.0], too_deep());
        assert_eq!(r.unwrap_err().code(), "WAL_CORRUPT");
        let r = e.insert_op(&s, "", VectorId(1), &[1.0, 0.0], Value::Null);
        assert_eq!(r.unwrap_err().code(), "WAL_CORRUPT", "bad op id");
        let r = e.insert(&s, VectorId(1), &[1.0], Value::Null);
        assert_eq!(r.unwrap_err().code(), "WAL_CORRUPT", "bad dimension");
        let r = e.delete(&s, VectorId(404));
        assert_eq!(r.unwrap_err().code(), "WAL_CORRUPT", "delete of absent id");
        let r = e.insert(&s, VectorId(1), &[f32::NAN, 0.0], Value::Null);
        assert_eq!(r.unwrap_err().code(), "WAL_CORRUPT", "non-finite vector");
        let big = Value::String("x".repeat(20 * 1024 * 1024));
        let r = e.insert_op(&s, "op", VectorId(1), &[1.0, 0.0], big);
        assert_eq!(r.unwrap_err().code(), "WAL_CORRUPT", "record too large");
        // Authorization still comes first.
        let r = e.insert(
            &EffectiveScope::default(),
            VectorId(1),
            &[1.0, 0.0],
            too_deep(),
        );
        assert_eq!(r.unwrap_err().code(), "FORBIDDEN");
    }

    #[test]
    fn append_rejects_when_already_poisoned() {
        // PIN of existing behaviour (not a race): `append` on an already-poisoned engine
        // rejects before writing. The real race is tests/review4.rs (pause hook + queued cases).
        let dir = std::env::temp_dir().join(format!("atlas-gate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let e = Engine::open(&dir, Config::new(2)).unwrap();
        let rec = Record {
            tenant: "a".into(),
            id: VectorId(1),
            vector: vec![1.0, 0.0],
            metadata: Value::Null,
            deleted: false,
            op_id: Some("op".into()),
        };
        let (payload, _) = e.prepare(&rec).unwrap();
        let len = std::fs::metadata(dir.join("wal.log")).unwrap().len();
        note_failure(&e.poison, Err(AtlasError::Codec("late".into())));
        assert_eq!(e.append(payload, true).unwrap_err().code(), "WAL_CORRUPT");
        assert_eq!(std::fs::metadata(dir.join("wal.log")).unwrap().len(), len);
        assert!(e.ops.lock().unwrap().is_empty());
    }

    /// Index that refuses id 666 (a failing `AnnIndex`), so the live applier poisons the engine.
    struct Refuses(FlatIndex);

    impl AnnIndex for Refuses {
        fn dim(&self) -> usize {
            self.0.dim()
        }
        fn len(&self) -> usize {
            self.0.len()
        }
        fn insert(&mut self, id: VectorId, v: &[f32], md: Value) -> Result<(), AtlasError> {
            if id.0 == 666 {
                return Err(AtlasError::Codec("index refused".into()));
            }
            self.0.insert(id, v, md)
        }
        fn entries(&self) -> Vec<(VectorId, Vec<f32>, Value)> {
            self.0.entries()
        }
        fn remove(&mut self, id: VectorId) -> bool {
            self.0.remove(id)
        }
        fn search(&self, q: &[f32], k: usize, m: Metric) -> Result<Vec<Hit>, AtlasError> {
            self.0.search(q, k, m)
        }
    }

    /// Deterministic race: T (insert_op) passes the cheap poison check and prepares its payload,
    /// then is paused right before `op_gate`. The engine is poisoned through the supported
    /// failure path while T is paused; when released, T must be rejected without touching the
    /// WAL, a seq, the op map or the index, and without any acknowledgement.
    #[test]
    fn poison_between_cheap_check_and_gate_rejects_the_paused_insert_op() {
        use std::sync::mpsc::channel;
        let dir = std::env::temp_dir().join(format!("atlas-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let make = |d| -> BoxedIndex { Box::new(Refuses(FlatIndex::new(d))) };
        let e = Arc::new(Engine::open_with_index(&dir, Config::new(2), make).unwrap());
        let s = scope("a");
        e.insert(&s, VectorId(1), &[1.0, 0.0], Value::Null).unwrap();
        let (reached_tx, reached_rx) = channel();
        let (go_tx, go_rx) = channel::<()>();
        let (reached_tx, go_rx) = (Mutex::new(reached_tx), Mutex::new(go_rx));
        e.set_before_gate_hook(move || {
            if std::thread::current().name() == Some("paused-insert-op") {
                reached_tx.lock().unwrap().send(()).unwrap();
                go_rx.lock().unwrap().recv().unwrap();
            }
        });
        let (e2, s2) = (e.clone(), s.clone());
        let t = std::thread::Builder::new()
            .name("paused-insert-op".into())
            .spawn(move || e2.insert_op(&s2, "late", VectorId(7), &[1.0, 0.0], Value::Null))
            .unwrap();
        reached_rx
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("T reached the pause point");
        // Poison via the supported failure path (applier rejects id 666), then wait for it.
        let culprit = e.insert(&s, VectorId(666), &[1.0, 0.0], Value::Null);
        assert_eq!(culprit.unwrap_err().code(), "WAL_CORRUPT");
        let wal_len = || std::fs::metadata(dir.join("wal.log")).unwrap().len();
        let len_poisoned = wal_len();
        let count = e.count(&s).unwrap();
        go_tx.send(()).unwrap();
        let r = t.join().unwrap();
        assert_eq!(r.unwrap_err().code(), "WAL_CORRUPT", "nothing acknowledged");
        assert_eq!(wal_len(), len_poisoned, "T wrote no bytes");
        assert!(e.ops.lock().unwrap().is_empty(), "no op-map entry for T");
        assert_eq!(e.count(&s).unwrap(), count, "count unchanged");
        drop(e);
        // Healthy index after reopen: T's record is absent and consumed no seq (1, 666 -> next 3).
        let e = Engine::open(&dir, Config::reopen()).unwrap();
        assert_eq!(e.count(&s).unwrap(), 2);
        let seq = e
            .insert_op(&s, "late", VectorId(7), &[1.0, 0.0], Value::Null)
            .unwrap();
        assert_eq!(seq, 3);
    }

    static FP_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn counting_fp(payload: &[u8]) -> u64 {
        FP_CALLS.fetch_add(1, Ordering::SeqCst);
        fingerprint(payload)
    }

    /// `prepare` is the only caller of the fingerprint fn (with encode + admit): a poisoned engine
    /// must reject before any of that work, including for would-be RECORD_TOO_LARGE inputs.
    #[test]
    fn poisoned_engine_rejects_before_any_encoding_work() {
        let dir = std::env::temp_dir().join(format!("atlas-fpcount-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut cfg = Config::new(2);
        cfg.max_record_len = 256;
        let e = Engine::open_with_fingerprint(&dir, cfg, counting_fp).unwrap();
        let s = scope("a");
        // Control: a healthy engine does reach the fingerprint.
        e.insert_op(&s, "ok", VectorId(1), &[1.0, 0.0], Value::Null)
            .unwrap();
        // prepare (live) and the applier (replay_one) each fingerprint once.
        let base = FP_CALLS.load(Ordering::SeqCst);
        assert!(
            base >= 1,
            "control: a healthy write reaches the fingerprint"
        );
        note_failure(&e.poison, Err(AtlasError::Codec("boom".into())));
        let valid = e.insert_op(&s, "op", VectorId(2), &[1.0, 0.0], Value::Null);
        assert_eq!(valid.unwrap_err().code(), "WAL_CORRUPT");
        let big = Value::String("x".repeat(1024));
        let too_large = e.insert_op(&s, "op2", VectorId(3), &[1.0, 0.0], big);
        assert_eq!(too_large.unwrap_err().code(), "WAL_CORRUPT");
        let nan = e.insert_op(&s, "op3", VectorId(4), &[f32::NAN, 0.0], Value::Null);
        assert_eq!(nan.unwrap_err().code(), "WAL_CORRUPT");
        assert_eq!(
            FP_CALLS.load(Ordering::SeqCst),
            base,
            "no encode/admit/fp work"
        );
    }

    #[test]
    fn lookup_op_releases_the_op_map_lock_before_the_caller_compares() {
        let dir = std::env::temp_dir().join(format!("atlas-lookup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let e = Engine::open(&dir, Config::new(2)).unwrap();
        let s = scope("a");
        e.insert_op(&s, "op", VectorId(1), &[1.0, 0.0], Value::Null)
            .unwrap();
        let t = TenantId::parse("a").unwrap();
        let held = e.lookup_op(&t, "op").unwrap().expect("entry");
        // While the caller holds the copied-out entry (and would compare it), the map is free.
        assert!(e.ops.try_lock().is_ok(), "ops mutex must not be held");
        assert!(e.lookup_op(&t, "absent").unwrap().is_none());
        drop(held);
    }

    #[test]
    fn canonical_encoding_sorts_object_keys() {
        let ab: Value = serde_json::from_str(r#"{"b":1,"a":2}"#).unwrap();
        let ba: Value = serde_json::from_str(r#"{"a":2,"b":1}"#).unwrap();
        assert_eq!(serde_json::to_vec(&ab).unwrap(), br#"{"a":2,"b":1}"#);
        assert_eq!(
            serde_json::to_vec(&ab).unwrap(),
            serde_json::to_vec(&ba).unwrap()
        );
        let rec = |md| Record {
            tenant: "a".into(),
            id: VectorId(1),
            vector: vec![1.0, 0.0],
            metadata: md,
            deleted: false,
            op_id: Some("op".into()),
        };
        assert_eq!(
            serde_json::to_vec(&rec(ab)).unwrap(),
            serde_json::to_vec(&rec(ba)).unwrap(),
            "record bytes are independent of source key order"
        );
    }

    #[test]
    fn live_retry_bytes_equal_replay_reconstructed_bytes() {
        let dir = std::env::temp_dir().join(format!("atlas-bytes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let e = Engine::open(&dir, Config::new(2)).unwrap();
        let s = scope("a");
        let md = serde_json::json!({"k": [1.5, "x", {"z": null}]});
        e.insert_op(&s, "op", VectorId(1), &[1.0, 0.5], md.clone())
            .unwrap();
        let retry = Record {
            tenant: "a".into(),
            id: VectorId(1),
            vector: vec![1.0, 0.5],
            metadata: md,
            deleted: false,
            op_id: Some("op".into()),
        };
        let (live, _) = e.prepare(&retry).unwrap();
        let key = (TenantId::parse("a").unwrap(), "op".to_string());
        let stored_live = e.ops.lock().unwrap()[&key].payload.to_vec();
        drop(e);
        let e = Engine::open(&dir, Config::reopen()).unwrap();
        let stored_replay = e.ops.lock().unwrap()[&key].payload.to_vec();
        assert_eq!(live, stored_live);
        assert_eq!(live, stored_replay);
    }
}
