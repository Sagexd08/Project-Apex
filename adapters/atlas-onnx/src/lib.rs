use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tract_onnx::prelude::*;

pub const SEQ: usize = 128;
pub const DIM: usize = 384;
const MAX_WORD_CHARS: usize = 100;

pub struct WordPiece {
    vocab: HashMap<String, i64>,
    cls: i64,
    sep: i64,
    unk: i64,
}

impl WordPiece {
    pub fn from_vocab(text: &str) -> Result<Self, String> {
        let vocab: HashMap<String, i64> = text
            .lines()
            .enumerate()
            .map(|(i, l)| (l.trim_end().to_string(), i as i64))
            .collect();
        let special = |t: &str| {
            vocab
                .get(t)
                .copied()
                .ok_or_else(|| format!("vocab is missing {t}"))
        };
        let (cls, sep, unk) = (special("[CLS]")?, special("[SEP]")?, special("[UNK]")?);
        Ok(WordPiece {
            vocab,
            cls,
            sep,
            unk,
        })
    }

    fn words(text: &str) -> Vec<String> {
        let mut out = vec![];
        let mut cur = String::new();
        for ch in text.to_lowercase().chars() {
            if ch.is_control() {
                continue;
            }
            if ch.is_whitespace() {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            } else if ch.is_alphanumeric() {
                cur.push(ch);
            } else {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                out.push(ch.to_string());
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
        out
    }

    fn pieces(&self, word: &str, out: &mut Vec<i64>) {
        let chars: Vec<char> = word.chars().collect();
        if chars.len() > MAX_WORD_CHARS {
            out.push(self.unk);
            return;
        }
        let mut found = vec![];
        let mut start = 0;
        while start < chars.len() {
            let mut end = chars.len();
            let mut hit = None;
            while end > start {
                let piece: String = chars[start..end].iter().collect();
                let key = if start > 0 {
                    format!("##{piece}")
                } else {
                    piece
                };
                if let Some(&id) = self.vocab.get(&key) {
                    hit = Some((id, end));
                    break;
                }
                end -= 1;
            }
            match hit {
                Some((id, e)) => {
                    found.push(id);
                    start = e;
                }
                None => {
                    out.push(self.unk);
                    return;
                }
            }
        }
        out.extend(found);
    }

    pub fn encode(&self, text: &str) -> (Vec<i64>, Vec<i64>) {
        let mut ids = vec![self.cls];
        for w in Self::words(text) {
            self.pieces(&w, &mut ids);
            if ids.len() >= SEQ - 1 {
                break;
            }
        }
        ids.truncate(SEQ - 1);
        ids.push(self.sep);
        let mut mask = vec![1i64; ids.len()];
        ids.resize(SEQ, 0);
        mask.resize(SEQ, 0);
        (ids, mask)
    }
}

type Plan = Arc<TypedSimplePlan>;

pub struct Embedder {
    tokenizer: WordPiece,
    plan: Plan,
}

fn msg(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Embedder {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let vocab = std::fs::read_to_string(dir.join("vocab.txt")).map_err(msg)?;
        let tokenizer = WordPiece::from_vocab(&vocab)?;
        let fact = || InferenceFact::dt_shape(i64::datum_type(), tvec!(1usize, SEQ));
        let plan = tract_onnx::onnx()
            .model_for_path(dir.join("model.onnx"))
            .map_err(msg)?
            .with_input_fact(0, fact())
            .map_err(msg)?
            .with_input_fact(1, fact())
            .map_err(msg)?
            .with_input_fact(2, fact())
            .map_err(msg)?
            .into_optimized()
            .map_err(msg)?
            .into_runnable()
            .map_err(msg)?;
        Ok(Embedder { tokenizer, plan })
    }

    pub fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let (ids, mask) = self.tokenizer.encode(text);
        let tensor = |v: Vec<i64>| -> Result<TValue, String> {
            let a = tract_ndarray::Array2::from_shape_vec((1, SEQ), v).map_err(msg)?;
            Ok(Tensor::from(a).into())
        };
        let inputs = tvec!(
            tensor(ids)?,
            tensor(mask.clone())?,
            tensor(vec![0i64; SEQ])?
        );
        let outputs = self.plan.run(inputs).map_err(msg)?;
        let hidden = outputs[0]
            .to_plain_array_view::<f32>()
            .map_err(msg)?
            .into_dimensionality::<tract_ndarray::Ix3>()
            .map_err(msg)?;
        let mut pooled = vec![0f32; DIM];
        let mut count = 0f32;
        for s in 0..SEQ {
            if mask[s] == 1 {
                for (d, p) in pooled.iter_mut().enumerate() {
                    *p += hidden[[0, s, d]];
                }
                count += 1.0;
            }
        }
        let norm = pooled
            .iter()
            .map(|x| (x / count) * (x / count))
            .sum::<f32>()
            .sqrt()
            .max(1e-12);
        Ok(pooled.into_iter().map(|x| (x / count) / norm).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn models() -> std::path::PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
        assert!(
            dir.join("model.onnx").exists() && dir.join("vocab.txt").exists(),
            "download the model files into {}",
            dir.display()
        );
        dir
    }

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }

    #[test]
    fn tokenizer_matches_known_bert_ids() {
        let vocab = std::fs::read_to_string(models().join("vocab.txt")).unwrap();
        let tok = WordPiece::from_vocab(&vocab).unwrap();
        let (ids, mask) = tok.encode("Hello, world!");
        assert_eq!(&ids[..6], &[101, 7592, 1010, 2088, 999, 102]);
        assert_eq!(mask.iter().sum::<i64>(), 6);
        assert_eq!(ids.len(), SEQ);
        let (long, _) = tok.encode(&"word ".repeat(500));
        assert_eq!(long[SEQ - 1], 102);
    }

    #[test]
    fn embeddings_are_unit_length_and_deterministic() {
        let e = Embedder::load(&models()).unwrap();
        let a = e.embed("write ahead log durability").unwrap();
        let b = e.embed("write ahead log durability").unwrap();
        assert_eq!(a.len(), DIM);
        assert_eq!(a, b);
        assert!((cosine(&a, &a) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn related_sentences_outscore_unrelated_with_no_shared_keywords() {
        let e = Embedder::load(&models()).unwrap();
        let q = e.embed("How do we make sure data survives a power cut?").unwrap();
        let related = e
            .embed("Acknowledged writes are flushed to disk before the call returns.")
            .unwrap();
        let unrelated = e.embed("The marketing team approved the quarterly budget.").unwrap();
        assert!(cosine(&q, &related) > cosine(&q, &unrelated) + 0.1);
    }
}
