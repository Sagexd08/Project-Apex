//! Deterministic document ingestion (no LLM). A document is Markdown with optional front matter:
//!
//! ```text
//! ---
//! id: adr-011
//! kind: decision       (default: artifact)
//! title: Adopt an event log
//! status: confirmed    (asserted observed inferred confirmed disputed; default asserted)
//! date: 2026-09-14
//! parent: proj-orders  (parent CONTAINS this)
//! supersedes: adr-007  (this SUPERSEDES that)
//! derived_from: exp-1, obs-2
//! caused_by: [obs-2]   (also: supports, disputes, depends_on, relates_to)
//! ---
//! Body text. [[other-id]] links become RELATES_TO relationships.
//! ```
//! Provenance is real: the source of every object and relationship is `file:<path>#sha256:<hash>`
//! of the exact document text. Re-ingesting identical text is a no-op; changed text for an existing
//! id is reported as a conflict (write a new id with `supersedes:` instead).
use crate::model::*;
use crate::store::Lcf;
use atlas_core::EffectiveScope;
use serde_json::{json, Value};

pub const MAX_DOCS: usize = 500;
pub const MAX_DOC_BYTES: usize = 256 * 1024;
const REL_KEYS: [(&str, &str); 6] = [
    ("supports", "SUPPORTS"),
    ("disputes", "DISPUTES"),
    ("derived_from", "DERIVED_FROM"),
    ("caused_by", "CAUSED_BY"),
    ("depends_on", "DEPENDS_ON"),
    ("relates_to", "RELATES_TO"),
];

#[derive(Clone)]
pub struct Doc {
    pub path: String,
    pub text: String,
}

const WORKERS: usize = 8;

/// Run `f` over `items` on a few threads (so durable writes overlap and the engine can group
/// commit them); results come back in input order.
pub fn par_map<'a, T: Sync, R: Send + 'a>(items: &'a [T], f: impl Fn(&'a T) -> R + Sync) -> Vec<R> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let out = std::sync::Mutex::new(Vec::with_capacity(items.len()));
    std::thread::scope(|sc| {
        for _ in 0..WORKERS.min(items.len().max(1)) {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if i >= items.len() {
                    break;
                }
                let r = f(&items[i]);
                out.lock().unwrap_or_else(|e| e.into_inner()).push((i, r));
            });
        }
    });
    let mut v = out.into_inner().unwrap_or_else(|e| e.into_inner());
    v.sort_by_key(|x| x.0);
    v.into_iter().map(|x| x.1).collect()
}

struct Parsed {
    node: Node,
    edges: Vec<(String, String, String)>, // (from, rel, to)
    warnings: Vec<String>,
}

pub fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[4 * i],
                chunk[4 * i + 1],
                chunk[4 * i + 2],
                chunk[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

/// Days since 1970-01-01 for a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn parse_date(s: &str) -> Option<i64> {
    let p: Vec<&str> = s.trim().split('-').collect();
    if p.len() != 3 {
        return None;
    }
    let (y, m, d) = (
        p[0].parse::<i64>().ok()?,
        p[1].parse::<i64>().ok()?,
        p[2].parse::<i64>().ok()?,
    );
    ((1..=12).contains(&m) && (1..=31).contains(&d) && (1970..=2200).contains(&y))
        .then(|| days_from_civil(y, m, d) * 86_400)
}

fn list(v: &str) -> Vec<String> {
    v.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|x| x.trim().trim_matches('"').to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

fn slug(s: &str) -> String {
    let t: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    t.split('-')
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn split_front(text: &str) -> (Vec<(String, String)>, &str) {
    let t = text.trim_start_matches('\u{feff}');
    let Some(rest) = t.strip_prefix("---") else {
        return (vec![], t);
    };
    let rest = rest.trim_start_matches(['\r', '\n']);
    let Some(end) = rest.find("\n---") else {
        return (vec![], t);
    };
    let head = &rest[..end];
    let body = rest[end + 4..].trim_start_matches(['\r', '\n']);
    let kv = head
        .lines()
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string()))
        .collect();
    (kv, body)
}

fn parse_doc(doc: &Doc) -> Result<Parsed, String> {
    if doc.text.len() > MAX_DOC_BYTES {
        return Err(format!("document larger than {MAX_DOC_BYTES} bytes"));
    }
    let (kv, body) = split_front(&doc.text);
    let get = |k: &str| kv.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
    let stem = doc.path.rsplit(['/', '\\']).next().unwrap_or(&doc.path);
    let stem = stem.rsplit_once('.').map(|(a, _)| a).unwrap_or(stem);
    let key = get("id")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| slug(stem));
    let heading = body
        .lines()
        .find_map(|l| l.strip_prefix("# "))
        .map(|s| s.trim().to_string());
    let title = get("title")
        .filter(|s| !s.is_empty())
        .or(heading)
        .unwrap_or_else(|| stem.to_string());
    let mut warnings = vec![];
    let ts = match get("date") {
        Some(d) => parse_date(&d).unwrap_or_else(|| {
            warnings.push(format!(
                "{key}: unreadable date {d:?}, stored as undated (0)"
            ));
            0
        }),
        None => 0,
    };
    let hash = sha256_hex(doc.text.as_bytes());
    let source = format!("file:{}#sha256:{hash}", doc.path.replace('\\', "/"));
    let body_text: String = if body.chars().count() > MAX_BODY {
        warnings.push(format!(
            "{key}: body truncated to {MAX_BODY} chars (full text hash is in source)"
        ));
        body.chars().take(MAX_BODY - 1).collect::<String>() + "…"
    } else {
        body.to_string()
    };
    let node = Node {
        key: key.clone(),
        kind: get("kind")
            .unwrap_or_else(|| "artifact".into())
            .to_lowercase(),
        title: title.chars().take(MAX_TITLE).collect(),
        body: body_text,
        source,
        status: get("status")
            .unwrap_or_else(|| "asserted".into())
            .to_lowercase(),
        ts,
        ver: 0,
    };
    let mut edges = vec![];
    if let Some(p) = get("parent") {
        for x in list(&p) {
            edges.push((x, "CONTAINS".to_string(), key.clone()));
        }
    }
    if let Some(p) = get("supersedes") {
        for x in list(&p) {
            edges.push((key.clone(), "SUPERSEDES".to_string(), x));
        }
    }
    for (field, rel) in REL_KEYS {
        if let Some(v) = get(field) {
            for x in list(&v) {
                edges.push((key.clone(), rel.to_string(), x));
            }
        }
    }
    let mut rest = body;
    while let Some(i) = rest.find("[[") {
        let Some(j) = rest[i + 2..].find("]]") else {
            break;
        };
        let target = rest[i + 2..i + 2 + j].trim().to_string();
        if !target.is_empty() && target != key {
            edges.push((key.clone(), "RELATES_TO".to_string(), target));
        }
        rest = &rest[i + 2 + j + 2..];
    }
    edges.sort();
    edges.dedup();
    Ok(Parsed {
        node,
        edges,
        warnings,
    })
}

impl<'a> Lcf<'a> {
    /// Ingest a batch of documents. Nodes first, then relationships; relationships whose target is
    /// not (yet) known are reported as `unresolved` and are completed by ingesting the target.
    pub fn ingest(&self, sc: &EffectiveScope, docs: &[Doc]) -> Res<Value> {
        if docs.is_empty() || docs.len() > MAX_DOCS {
            return invalid("documents must be 1..=500");
        }
        let before = self.current_version(sc)?;
        let (mut written, mut unchanged) = (0usize, 0usize);
        let (mut conflicts, mut errors, mut warnings) = (vec![], vec![], vec![]);
        let mut parsed = vec![];
        for d in docs {
            match parse_doc(d) {
                Ok(p) => parsed.push((d.path.clone(), p)),
                Err(e) => errors.push(json!({"document": d.path, "error": e})),
            }
        }
        let mut ok_keys = std::collections::HashSet::new();
        let node_results = par_map(&parsed, |(path, p)| (path, p, self.put_node(sc, &p.node)));
        for (path, p, res) in node_results {
            warnings.extend(p.warnings.iter().cloned());
            match res {
                Ok(v) => {
                    if v > before {
                        written += 1;
                    } else {
                        unchanged += 1;
                    }
                    ok_keys.insert(p.node.key.clone());
                }
                Err(LcfError::Conflict(m)) => conflicts.push(json!({
                    "document": path, "id": p.node.key, "detail": m,
                    "hint": "the text of this id changed; give the new text a new id with supersedes: <old id>"})),
                Err(LcfError::Invalid(m)) => {
                    errors.push(json!({"document": path, "id": p.node.key, "error": m}))
                }
                Err(e) => return Err(e),
            }
        }
        let before_edges = self.current_version(sc)?;
        let mut work: Vec<(&String, &Parsed, &(String, String, String))> = vec![];
        for (path, p) in &parsed {
            if ok_keys.contains(&p.node.key) {
                work.extend(p.edges.iter().map(|e| (path, p, e)));
            }
        }
        let (mut e_written, mut e_unchanged, mut unresolved) = (0usize, 0usize, vec![]);
        let edge_results = par_map(&work, |(path, p, (from, rel, to))| {
            let e = Edge {
                rel: rel.clone(),
                from: from.clone(),
                to: to.clone(),
                source: p.node.source.clone(),
                ts: p.node.ts,
                status: p.node.status.clone(),
                ver: 0,
            };
            (*path, from, rel, to, self.put_edge(sc, &e))
        });
        for (path, from, rel, to, res) in edge_results {
            match res {
                Ok(v) if v > before_edges => e_written += 1,
                Ok(_) => e_unchanged += 1,
                Err(LcfError::NotFound(_)) => {
                    unresolved.push(json!({"document": path, "from": from, "rel": rel, "to": to}))
                }
                Err(LcfError::Conflict(m)) => {
                    conflicts.push(json!({"document": path, "edge": [from, rel, to], "detail": m}))
                }
                Err(LcfError::Invalid(m)) => {
                    errors.push(json!({"document": path, "edge": [from, rel, to], "error": m}))
                }
                Err(e) => return Err(e),
            }
        }
        Ok(json!({
            "nodes_written": written, "nodes_unchanged": unchanged,
            "relationships_written": e_written, "relationships_unchanged": e_unchanged,
            "unresolved": unresolved, "conflicts": conflicts, "errors": errors, "warnings": warnings,
            "version": self.current_version(sc)?,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn dates_parse_to_epoch_seconds() {
        assert_eq!(parse_date("1970-01-02"), Some(86_400));
        assert_eq!(parse_date("2000-03-01"), Some(951_868_800));
        assert_eq!(parse_date("nope"), None);
    }

    #[test]
    fn front_matter_and_wiki_links_become_relationships() {
        let d = Doc {
            path: "docs/adr-011.md".into(),
            text: "---\nid: adr-011\nkind: decision\nparent: proj\nsupersedes: adr-007\nderived_from: [exp-1, obs-2]\n---\n# Adopt it\nSee [[other]] and [[other]].".into(),
        };
        let p = parse_doc(&d).unwrap();
        assert_eq!(p.node.key, "adr-011");
        assert_eq!(p.node.title, "Adopt it");
        let e: Vec<String> = p
            .edges
            .iter()
            .map(|(f, r, t)| format!("{f} {r} {t}"))
            .collect();
        assert!(e.contains(&"proj CONTAINS adr-011".to_string()));
        assert!(e.contains(&"adr-011 SUPERSEDES adr-007".to_string()));
        assert!(e.contains(&"adr-011 DERIVED_FROM exp-1".to_string()));
        assert_eq!(
            e.iter().filter(|x| x.ends_with("RELATES_TO other")).count(),
            1
        );
        assert!(p.node.source.starts_with("file:docs/adr-011.md#sha256:"));
    }

    #[test]
    fn plain_markdown_gets_a_slug_id_and_heading_title() {
        let p = parse_doc(&Doc {
            path: "notes/My Notes.md".into(),
            text: "# Hello\nbody".into(),
        })
        .unwrap();
        assert_eq!(
            (
                p.node.key.as_str(),
                p.node.title.as_str(),
                p.node.kind.as_str()
            ),
            ("my-notes", "Hello", "artifact")
        );
    }
}
