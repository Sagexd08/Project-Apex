//! Bench helpers: synthetic data, statistics, tiny JSON writer, FNV-1a, parallel map.

use crate::rng::SplitMix64;

/// `nc` Gaussian cluster centres of dimension `d`.
pub fn centres(rng: &mut SplitMix64, nc: usize, d: usize) -> Vec<Vec<f32>> {
    (0..nc).map(|_| (0..d).map(|_| rng.gauss() as f32).collect()).collect()
}

/// `n` unit vectors: random centre + `noise` * N(0,1) (+ `shift`) per component, L2-normalised.
pub fn mixture(rng: &mut SplitMix64, cs: &[Vec<f32>], n: usize, noise: f32, shift: f32) -> Vec<Vec<f32>> {
    (0..n)
        .map(|_| {
            let c = &cs[rng.below(cs.len())];
            let mut v: Vec<f32> = c.iter().map(|x| x + noise * rng.gauss() as f32 + shift).collect();
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            v.iter_mut().for_each(|x| *x /= norm);
            v
        })
        .collect()
}

/// Mean of a slice (0 if empty).
pub fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

/// Nearest-rank percentile `p` in `[0,100]` (sorts a copy).
pub fn pct(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let rank = ((p / 100.0) * s.len() as f64).ceil() as usize;
    s[rank.clamp(1, s.len()) - 1]
}

/// FNV-1a 64-bit hash.
pub fn fnv1a(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Evaluate `f(0..n)` on all cores, results in index order.
pub fn par_map<T: Send, F: Fn(usize) -> T + Sync>(n: usize, f: F) -> Vec<T> {
    let threads = std::thread::available_parallelism().map_or(1, |x| x.get());
    let mut out: Vec<(usize, T)> = std::thread::scope(|sc| {
        let f = &f;
        let hs: Vec<_> = (0..threads)
            .map(|t| sc.spawn(move || (t..n).step_by(threads).map(|i| (i, f(i))).collect::<Vec<_>>()))
            .collect();
        hs.into_iter().flat_map(|h| h.join().expect("worker panicked")).collect()
    });
    out.sort_by_key(|x| x.0);
    out.into_iter().map(|x| x.1).collect()
}

/// Minimal JSON value.
pub enum J {
    /// Number (non-finite renders as null).
    Num(f64),
    /// String.
    Str(String),
    /// Boolean.
    Bool(bool),
    /// Array.
    Arr(Vec<J>),
    /// Object (insertion order).
    Obj(Vec<(String, J)>),
}

/// Build a JSON object from `(key, value)` pairs.
pub fn obj(items: Vec<(&str, J)>) -> J {
    J::Obj(items.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

impl J {
    /// Render as compact JSON.
    pub fn render(&self) -> String {
        match self {
            J::Num(x) if x.is_finite() => format!("{x}"),
            J::Num(_) => "null".into(),
            J::Bool(b) => b.to_string(),
            J::Str(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")),
            J::Arr(a) => format!("[{}]", a.iter().map(J::render).collect::<Vec<_>>().join(",")),
            J::Obj(o) => {
                let items: Vec<String> = o.iter().map(|(k, v)| format!("\"{k}\":{}", v.render())).collect();
                format!("{{{}}}", items.join(","))
            }
        }
    }
}

/// `n` isotropic Gaussian unit vectors in `d` dimensions (no cluster structure).
pub fn isotropic(rng: &mut SplitMix64, n: usize, d: usize) -> Vec<Vec<f32>> {
    (0..n)
        .map(|_| {
            let mut v: Vec<f32> = (0..d).map(|_| rng.gauss() as f32).collect();
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            v.iter_mut().for_each(|x| *x /= norm);
            v
        })
        .collect()
}

/// Drop each carriage return that directly precedes a newline, so a digest names content, not a checkout.
pub fn fold_crlf(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    for (i, &c) in b.iter().enumerate() {
        if c == b'\r' && b.get(i + 1) == Some(&b'\n') {
            continue;
        }
        out.push(c);
    }
    out
}

fn rs_files_in(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).expect("read src dir") {
        let p = e.expect("dir entry").path();
        if p.is_dir() {
            rs_files_in(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// FNV-1a 64 over every `.rs` file under `dir` in sorted path order, with CRLF folded to LF.
pub fn src_digest_lf(dir: &std::path::Path) -> u64 {
    let mut names = Vec::new();
    rs_files_in(dir, &mut names);
    names.sort();
    let mut all = Vec::new();
    for n in names {
        all.extend(fold_crlf(&std::fs::read(n).expect("read file")));
    }
    fnv1a(&all)
}

/// FNV-1a 64 over the little-endian bytes of every `f32` in every set, in order.
pub fn dataset_digest(sets: &[&[Vec<f32>]]) -> u64 {
    let mut bytes = Vec::new();
    for set in sets {
        for v in set.iter() {
            for x in v {
                bytes.extend_from_slice(&x.to_le_bytes());
            }
        }
    }
    fnv1a(&bytes)
}
