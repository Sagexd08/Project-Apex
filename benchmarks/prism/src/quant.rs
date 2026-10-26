//! Nested 4-bit scalar quantiser stored as four bit planes (plane 0 = MSB).

/// Clip constant: coordinates are clipped to `+-CLIP/sqrt(P)`.
pub const CLIP: f32 = 3.0;
/// Number of stored bit planes.
pub const PLANES: usize = 4;

/// Half-range `s = CLIP / sqrt(p)`.
pub fn scale(p: usize) -> f32 {
    CLIP / (p as f32).sqrt()
}

/// 4-bit index of a rotated coordinate `z`.
pub fn idx4(z: f32, s: f32) -> u8 {
    let delta = 2.0 * s / 16.0;
    ((z + s) / delta).floor().clamp(0.0, 15.0) as u8
}

/// Directly quantise `z` with `b` bits (used to test the nested property).
pub fn idx_direct(z: f32, s: f32, b: usize) -> u8 {
    let levels = (1u32 << b) as f32;
    let delta = 2.0 * s / levels;
    ((z + s) / delta).floor().clamp(0.0, levels - 1.0) as u8
}

/// Reconstruction value of the top `b` bits of a 4-bit index.
pub fn recon(i4: u8, b: usize, s: f32) -> f32 {
    let idx = (i4 >> (PLANES - b)) as f32;
    let delta = 2.0 * s / (1u32 << b) as f32;
    (idx + 0.5) * delta - s
}

/// Reconstruction table for `b` bits: `table[idx_b]`.
pub fn table(b: usize, s: f32) -> Vec<f32> {
    let delta = 2.0 * s / (1u32 << b) as f32;
    (0..1u32 << b).map(|i| (i as f32 + 0.5) * delta - s).collect()
}

/// Append one rotated vector's 4 bit planes (`ceil(len/64)` words each).
pub fn push_encoded(z: &[f32], s: f32, planes: &mut [Vec<u64>; PLANES]) {
    let words = z.len().div_ceil(64);
    let start = planes[0].len();
    for pl in planes.iter_mut() {
        pl.resize(start + words, 0);
    }
    for (i, &zi) in z.iter().enumerate() {
        let q = idx4(zi, s);
        for (k, pl) in planes.iter_mut().enumerate() {
            let bit = ((q >> (PLANES - 1 - k)) & 1) as u64;
            pl[start + (i >> 6)] |= bit << (i & 63);
        }
    }
}

/// Decode coordinate `i` of the vector whose words start at `base`, at `b` bits.
pub fn decode_idx(planes: &[Vec<u64>; PLANES], base: usize, i: usize, b: usize) -> usize {
    let (w, sh) = (base + (i >> 6), i & 63);
    planes.iter().take(b).fold(0usize, |idx, pl| (idx << 1) | ((pl[w] >> sh) & 1) as usize)
}

/// Estimated score `sum_i q_i * recon_b(x)_i` for the vector starting at `base`.
pub fn score(q: &[f32], planes: &[Vec<u64>; PLANES], base: usize, b: usize, tab: &[f32]) -> f32 {
    match b {
        1 => score_n::<1>(q, planes, base, tab),
        2 => score_n::<2>(q, planes, base, tab),
        3 => score_n::<3>(q, planes, base, tab),
        4 => score_n::<4>(q, planes, base, tab),
        _ => panic!("bits must be 1..=4"),
    }
}

fn score_n<const B: usize>(q: &[f32], planes: &[Vec<u64>; PLANES], base: usize, tab: &[f32]) -> f32 {
    let mut acc = 0.0f32;
    for (w, chunk) in q.chunks(64).enumerate() {
        let mut ws = [0u64; B];
        for (k, x) in ws.iter_mut().enumerate() {
            *x = planes[k][base + w];
        }
        for (j, &qi) in chunk.iter().enumerate() {
            let idx = ws.iter().fold(0usize, |a, x| (a << 1) | ((x >> j) & 1) as usize);
            acc += qi * tab[idx];
        }
    }
    acc
}
