
use crate::quant::{self, PLANES};
use crate::rot::Rotation;
use std::sync::{Arc, Mutex, RwLock};

pub struct Epoch {
    pub seed: u64,
    pub dim: usize,
    pub p: usize,
    pub schedule: Vec<usize>,
    pub(crate) rot: Rotation,
    pub(crate) s: f32,
    pub(crate) words: usize,
    pub(crate) tabs: Vec<Vec<f32>>,
}

impl Epoch {
    pub fn new(seed: u64, dim: usize, schedule: Vec<usize>) -> Self {
        let rot = Rotation::new(dim, seed);
        let p = rot.padded_dim();
        let s = quant::scale(p);
        let tabs = (0..=PLANES).map(|b| if b == 0 { Vec::new() } else { quant::table(b, s) }).collect();
        Self { seed, dim, p, schedule, rot, s, words: p.div_ceil(64), tabs }
    }

    pub fn rotate(&self, v: &[f32]) -> Vec<f32> {
        self.rot.apply(v)
    }

    pub fn half_range(&self) -> f32 {
        self.s
    }
}

pub struct Segment {
    pub(crate) ids: Vec<u64>,
    pub(crate) vecs: Vec<f32>,
    pub(crate) planes: [Vec<u64>; PLANES],
    pub(crate) resid: Vec<f32>,
    pub(crate) dim: usize,
    pub(crate) words: usize,
    lsn_min: u64,
    lsn_max: u64,
}

impl Segment {
    fn build(ep: &Epoch, batch: &[(u64, Vec<f32>)], lsn: u64) -> Self {
        let mut seg = Segment {
            ids: Vec::with_capacity(batch.len()),
            vecs: Vec::with_capacity(batch.len() * ep.dim),
            planes: Default::default(),
            resid: Vec::with_capacity(batch.len() * PLANES),
            dim: ep.dim,
            words: ep.words,
            lsn_min: lsn,
            lsn_max: lsn,
        };
        for (id, v) in batch {
            assert_eq!(v.len(), ep.dim, "vector length != epoch dim");
            seg.ids.push(*id);
            seg.vecs.extend_from_slice(v);
            let z = ep.rot.apply(v);
            quant::push_encoded(&z, ep.s, &mut seg.planes);
            seg.resid.extend((1..=PLANES).map(|b| quant::residual(&z, ep.s, b)));
        }
        seg
    }

    fn merge(parts: &[Arc<Segment>]) -> Self {
        let mut seg = Segment {
            ids: Vec::new(),
            vecs: Vec::new(),
            planes: Default::default(),
            resid: Vec::new(),
            dim: parts[0].dim,
            words: parts[0].words,
            lsn_min: u64::MAX,
            lsn_max: 0,
        };
        for p in parts {
            seg.ids.extend_from_slice(&p.ids);
            seg.vecs.extend_from_slice(&p.vecs);
            seg.resid.extend_from_slice(&p.resid);
            for (dst, src) in seg.planes.iter_mut().zip(&p.planes) {
                dst.extend_from_slice(src);
            }
            seg.lsn_min = seg.lsn_min.min(p.lsn_min);
            seg.lsn_max = seg.lsn_max.max(p.lsn_max);
        }
        seg
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn ids(&self) -> &[u64] {
        &self.ids
    }

    pub fn lsn_range(&self) -> (u64, u64) {
        (self.lsn_min, self.lsn_max)
    }

    pub(crate) fn vec(&self, i: usize) -> &[f32] {
        &self.vecs[i * self.dim..(i + 1) * self.dim]
    }
}

pub struct Snapshot {
    pub lsn: u64,
    pub segments: Vec<Arc<Segment>>,
}

impl Snapshot {
    pub fn len(&self) -> usize {
        self.segments.iter().map(|s| s.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.segments.iter().all(|s| s.is_empty())
    }
}

pub struct Store {
    epoch: Epoch,
    current: RwLock<Arc<Snapshot>>,
    writer: Mutex<()>,
}

impl Store {
    pub fn new(epoch: Epoch) -> Self {
        let snap = Arc::new(Snapshot { lsn: 0, segments: Vec::new() });
        Self { epoch, current: RwLock::new(snap), writer: Mutex::new(()) }
    }

    pub fn epoch(&self) -> &Epoch {
        &self.epoch
    }

    pub fn pin(&self) -> Arc<Snapshot> {
        Arc::clone(&self.current.read().unwrap_or_else(|e| e.into_inner()))
    }

    fn publish(&self, snap: Snapshot) {
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(snap);
    }

    pub fn commit(&self, batch: Vec<(u64, Vec<f32>)>) -> u64 {
        let _w = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let old = self.pin();
        if batch.is_empty() {
            return old.lsn;
        }
        let lsn = old.lsn + 1;
        let seg = Arc::new(Segment::build(&self.epoch, &batch, lsn));
        let mut segments = old.segments.clone();
        segments.push(seg);
        self.publish(Snapshot { lsn, segments });
        lsn
    }

    pub fn compact(&self, small: usize) -> u64 {
        let _w = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let old = self.pin();
        let (parts, mut keep): (Vec<_>, Vec<_>) =
            old.segments.iter().cloned().partition(|s| s.len() <= small);
        if parts.len() < 2 {
            return old.lsn;
        }
        keep.push(Arc::new(Segment::merge(&parts)));
        let lsn = old.lsn + 1;
        self.publish(Snapshot { lsn, segments: keep });
        lsn
    }
}
