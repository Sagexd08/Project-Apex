use atlas_core::{Metric, VectorId};
use atlas_index::{AnnIndex, HnswIndex};
use numpy::{PyReadonlyArray1, PyReadonlyArray2};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use serde_json::Value;

fn parse_metric(name: &str) -> PyResult<Metric> {
    match name {
        "angular" | "cosine" => Ok(Metric::Cosine),
        "euclidean" => Ok(Metric::Euclidean),
        "dot" | "ip" => Ok(Metric::Dot),
        other => Err(PyValueError::new_err(format!("unknown metric {other}"))),
    }
}

fn fail(e: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(e.to_string())
}

#[pyclass]
struct Index {
    inner: HnswIndex,
    metric: Metric,
    dim: usize,
}

#[pymethods]
impl Index {
    #[new]
    #[pyo3(signature = (dim, metric = "angular", f16 = true, ef_search = 128, m = 32, ef_construction = 200))]
    fn new(
        dim: usize,
        metric: &str,
        f16: bool,
        ef_search: usize,
        m: usize,
        ef_construction: usize,
    ) -> PyResult<Self> {
        let metric = parse_metric(metric)?;
        let mut inner = HnswIndex::with_metric(dim, metric).with_graph_params(m, ef_construction);
        if f16 {
            inner = inner.with_f16_graph();
        }
        inner.set_ef_search(ef_search);
        Ok(Index { inner, metric, dim })
    }

    fn add(&mut self, py: Python<'_>, vectors: PyReadonlyArray2<f32>, first_id: u64) -> PyResult<()> {
        let view = vectors.as_array();
        if view.ncols() != self.dim {
            return Err(PyValueError::new_err("dimension mismatch"));
        }
        let rows: Vec<Vec<f32>> = view.rows().into_iter().map(|r| r.to_vec()).collect();
        let inner = &mut self.inner;
        py.allow_threads(|| {
            for (i, row) in rows.iter().enumerate() {
                inner
                    .insert(VectorId(first_id + i as u64), row, Value::Null)
                    .map_err(fail)?;
            }
            Ok(())
        })
    }

    fn set_ef_search(&mut self, ef: usize) {
        self.inner.set_ef_search(ef);
    }

    fn search(&self, query: PyReadonlyArray1<f32>, k: usize) -> PyResult<Vec<u64>> {
        let q = query.as_slice().map_err(fail)?;
        let hits = self.inner.search(q, k, self.metric).map_err(fail)?;
        Ok(hits.into_iter().map(|h| h.id.0).collect())
    }

    fn search_batch(
        &self,
        py: Python<'_>,
        queries: PyReadonlyArray2<f32>,
        k: usize,
    ) -> PyResult<Vec<Vec<u64>>> {
        let view = queries.as_array();
        let rows: Vec<Vec<f32>> = view.rows().into_iter().map(|r| r.to_vec()).collect();
        let (inner, metric) = (&self.inner, self.metric);
        py.allow_threads(|| {
            rows.iter()
                .map(|q| {
                    inner
                        .search(q, k, metric)
                        .map(|hits| hits.into_iter().map(|h| h.id.0).collect())
                        .map_err(fail)
                })
                .collect()
        })
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }
}

#[pymodule]
fn atlas_ann(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Index>()?;
    Ok(())
}
