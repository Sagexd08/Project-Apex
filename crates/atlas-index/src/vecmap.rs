use atlas_core::AtlasError;
use memmap2::Mmap;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone)]
pub struct MappedFile {
    map: Arc<Mmap>,
}

impl MappedFile {
    pub fn open(path: &Path) -> Result<Self, AtlasError> {
        let file = File::open(path)?;
        let map = unsafe { Mmap::map(&file) }?;
        Ok(Self { map: Arc::new(map) })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.map
    }

    pub fn floats(&self, byte_off: usize, count: usize) -> Result<VectorMap, AtlasError> {
        let bad = |why: &str| AtlasError::WalCorrupt(format!("vector map: {why}"));
        let len = count.checked_mul(4).ok_or_else(|| bad("length overflow"))?;
        let end = byte_off
            .checked_add(len)
            .ok_or_else(|| bad("range overflow"))?;
        if end > self.map.len() {
            return Err(bad("range past end"));
        }
        if !(self.map.as_ptr() as usize + byte_off).is_multiple_of(std::mem::align_of::<f32>()) {
            return Err(bad("misaligned"));
        }
        if cfg!(target_endian = "big") {
            return Err(bad("big-endian host"));
        }
        Ok(VectorMap {
            file: self.clone(),
            off: byte_off,
            count,
        })
    }
}

#[derive(Clone)]
pub struct VectorMap {
    file: MappedFile,
    off: usize,
    count: usize,
}

impl VectorMap {
    pub fn as_slice(&self) -> &[f32] {
        let base = self.file.map.as_ptr();
        unsafe { std::slice::from_raw_parts(base.add(self.off).cast::<f32>(), self.count) }
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}
