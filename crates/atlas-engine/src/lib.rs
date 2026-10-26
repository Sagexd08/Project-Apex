//! Embedded Atlas engine: insert -> WAL (durable ack) -> exact index.
use atlas_core::{validate_vector, AtlasError, Config, Metric, VectorId};
use atlas_index::{AnnIndex, FlatIndex};
use atlas_storage::{read_meta, write_meta, Applier, Wal};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex, RwLock};

pub use atlas_index::Hit;

#[derive(Serialize, Deserialize)]
struct Record {
    id: VectorId,
    vector: Vec<f32>,
    metadata: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    op_id: Option<String>,
}

/// The index is derived from the WAL and rebuilt on every open; any `AnnIndex` can be swapped in.
pub type BoxedIndex = Box<dyn AnnIndex + Send + Sync>;

const MAX_OP_ID_LEN: usize = 128;

/// (seq, payload fingerprint) per operation id.
/// ponytail: the dedupe window is the whole WAL and this map grows without bound; add
/// expiry/compaction of op ids when snapshots exist.
type OpMap = HashMap<String, (u64, u64)>;

/// FNV-1a 64 over the canonical request bytes (id, vector, metadata).
fn fingerprint(id: VectorId, vector: &[f32], metadata: &Value) -> u64 {
    let bytes = serde_json::to_vec(&(id, vector, metadata)).unwrap_or_default();
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100000001b3)
    })
}

fn note_op(ops: &Mutex<OpMap>, seq: u64, r: &Record) {
    if let Some(op) = &r.op_id {
        ops.lock()
            .expect("op map poisoned")
            .entry(op.clone())
            .or_insert((seq, fingerprint(r.id, &r.vector, &r.metadata)));
    }
}

pub struct Engine {
    dim: usize,
    wal: Wal,
    index: Arc<RwLock<BoxedIndex>>,
    ops: Arc<Mutex<OpMap>>,
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

impl Engine {
    /// Create (`cfg.dim` required) or reopen (`cfg.dim` optional; if given it must match
    /// the persisted dimension, else DIMENSION_MISMATCH).
    pub fn open(dir: impl AsRef<Path>, cfg: Config) -> Result<Engine, AtlasError> {
        Self::open_with_index(dir, cfg, |dim| Box::new(FlatIndex::new(dim)))
    }

    /// Like `open`, with a caller-chosen empty index built for the resolved dimension. The WAL
    /// is replayed into it; persisted index files are never trusted.
    pub fn open_with_index(
        dir: impl AsRef<Path>,
        cfg: Config,
        make_index: impl FnOnce(usize) -> BoxedIndex,
    ) -> Result<Engine, AtlasError> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let dim = resolve_dim(dir, cfg.dim)?;
        let index = Arc::new(RwLock::new(make_index(dim)));
        let applier_index = index.clone();
        let ops: Arc<Mutex<OpMap>> = Arc::new(Mutex::new(HashMap::new()));
        let applier_ops = ops.clone();
        let applier: Applier = Box::new(move |seq, payload| {
            // Payloads were produced by `insert` and validated before the WAL write.
            if let Ok(r) = serde_json::from_slice::<Record>(payload) {
                note_op(&applier_ops, seq, &r);
                let _ = applier_index
                    .write()
                    .expect("index lock poisoned")
                    .insert(r.id, &r.vector, r.metadata);
            }
        });
        let (wal, records) = Wal::open(&dir.join("wal.log"), &cfg, applier)?;
        {
            let mut ix = index.write().expect("index lock poisoned");
            for (seq, payload) in records {
                let r: Record = serde_json::from_slice(&payload)
                    .map_err(|e| AtlasError::WalCorrupt(format!("seq {seq}: {e}")))?;
                note_op(&ops, seq, &r);
                ix.insert(r.id, &r.vector, r.metadata)
                    .map_err(|e| AtlasError::WalCorrupt(format!("seq {seq}: {e}")))?;
            }
        }
        Ok(Engine {
            dim,
            wal,
            index,
            ops,
            op_gate: Mutex::new(()),
        })
    }

    /// Returns the WAL seq once the record reached the configured durability boundary
    /// and has been applied to the index (in WAL order, by the committer thread).
    pub fn insert(&self, id: VectorId, vector: &[f32], metadata: Value) -> Result<u64, AtlasError> {
        validate_vector(self.dim, vector)?;
        let payload = serde_json::to_vec(&Record {
            id,
            vector: vector.to_vec(),
            metadata,
            op_id: None,
        })
        .map_err(|e| AtlasError::Codec(e.to_string()))?;
        self.wal.append(payload)
    }

    /// Idempotent insert keyed by a caller-supplied operation id (1..=128 bytes).
    ///
    /// Retrying the same `op_id` with the same payload (after OUTCOME_UNKNOWN, a crash or a
    /// restart) never applies twice and returns the ORIGINAL seq. Same `op_id` with a different
    /// payload is OP_ID_CONFLICT. The dedupe window is the whole WAL (unbounded, see OpMap).
    pub fn insert_op(
        &self,
        op_id: &str,
        id: VectorId,
        vector: &[f32],
        metadata: Value,
    ) -> Result<u64, AtlasError> {
        if op_id.is_empty() || op_id.len() > MAX_OP_ID_LEN {
            return Err(AtlasError::InvalidOpId);
        }
        validate_vector(self.dim, vector)?;
        let fp = fingerprint(id, vector, &metadata);
        let _gate = self.op_gate.lock().expect("op gate poisoned");
        let known = self
            .ops
            .lock()
            .expect("op map poisoned")
            .get(op_id)
            .copied();
        if let Some((seq, known_fp)) = known {
            return if known_fp == fp {
                Ok(seq)
            } else {
                Err(AtlasError::OpIdConflict(op_id.to_string()))
            };
        }
        let payload = serde_json::to_vec(&Record {
            id,
            vector: vector.to_vec(),
            metadata,
            op_id: Some(op_id.to_string()),
        })
        .map_err(|e| AtlasError::Codec(e.to_string()))?;
        self.wal.append(payload)
    }

    pub fn search(&self, q: &[f32], k: usize, metric: Metric) -> Result<Vec<Hit>, AtlasError> {
        self.index
            .read()
            .expect("index lock poisoned")
            .search(q, k, metric)
    }

    /// Top-k among ids accepted by `allow` (exact for the default index).
    pub fn search_filtered(
        &self,
        q: &[f32],
        k: usize,
        metric: Metric,
        allow: &dyn Fn(VectorId) -> bool,
    ) -> Result<Vec<Hit>, AtlasError> {
        self.index
            .read()
            .expect("index lock poisoned")
            .search_filtered(q, k, metric, allow)
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn len(&self) -> usize {
        self.index.read().expect("index lock poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[doc(hidden)]
    pub fn fail_next_flush_handle(&self) -> Arc<AtomicBool> {
        self.wal.fail_next_flush_handle()
    }

    #[doc(hidden)]
    pub fn fail_next_truncate_handle(&self) -> Arc<AtomicBool> {
        self.wal.fail_next_truncate_handle()
    }

    #[doc(hidden)]
    pub fn fail_next_zero_handle(&self) -> Arc<AtomicBool> {
        self.wal.fail_next_zero_handle()
    }

    #[doc(hidden)]
    pub fn partial_write_handle(&self) -> Arc<AtomicBool> {
        self.wal.partial_write_handle()
    }

    #[doc(hidden)]
    pub fn fail_next_marker_handle(&self) -> Arc<AtomicBool> {
        self.wal.fail_next_marker_handle()
    }

    #[doc(hidden)]
    pub fn die_after_marker_handle(&self) -> Arc<AtomicBool> {
        self.wal.die_after_marker_handle()
    }

    #[doc(hidden)]
    pub fn flush_delay_handle(&self) -> Arc<AtomicU64> {
        self.wal.flush_delay_handle()
    }
}
