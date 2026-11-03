//! Logical model. Every object and every edge is one canonical engine record (immutable by
//! convention: a change is a new object plus a SUPERSEDES edge, never an overwrite).
use atlas_core::VectorId;
use serde_json::{json, Value};

pub const DIM: usize = 64;
pub const MAX_KEY: usize = 128;
pub const MAX_TITLE: usize = 200;
pub const MAX_BODY: usize = 4000;

pub const KINDS: [&str; 11] = [
    "workspace",
    "project",
    "agent",
    "episode",
    "observation",
    "entity",
    "fact",
    "decision",
    "experiment",
    "artifact",
    "evidence",
];
pub const RELS: [&str; 8] = [
    "CONTAINS",
    "RELATES_TO",
    "SUPPORTS",
    "DISPUTES",
    "DERIVED_FROM",
    "CAUSED_BY",
    "DEPENDS_ON",
    "SUPERSEDES",
];
pub const STATUSES: [&str; 5] = ["asserted", "observed", "inferred", "confirmed", "disputed"];

#[derive(Debug)]
pub enum LcfError {
    Invalid(String),
    NotFound(String),
    Conflict(String),
    Unavailable(String),
    Engine(atlas_core::AtlasError),
}

impl LcfError {
    pub fn code(&self) -> String {
        match self {
            LcfError::Invalid(_) => "INVALID".into(),
            LcfError::NotFound(_) => "NOT_FOUND".into(),
            LcfError::Conflict(_) => "CONFLICT".into(),
            LcfError::Unavailable(_) => "UNAVAILABLE".into(),
            LcfError::Engine(e) => e.code().to_string(),
        }
    }
    pub fn detail(&self) -> String {
        match self {
            LcfError::Invalid(s)
            | LcfError::NotFound(s)
            | LcfError::Conflict(s)
            | LcfError::Unavailable(s) => s.clone(),
            LcfError::Engine(_) => String::new(),
        }
    }
}

impl From<atlas_core::AtlasError> for LcfError {
    fn from(e: atlas_core::AtlasError) -> Self {
        LcfError::Engine(e)
    }
}

pub type Res<T> = Result<T, LcfError>;

pub fn invalid<T>(m: &str) -> Res<T> {
    Err(LcfError::Invalid(m.to_string()))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub key: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub source: String,
    pub status: String,
    pub ts: i64,
    /// Version assigned by the store at write time (0 = not yet written / legacy record).
    pub ver: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Edge {
    pub rel: String,
    pub from: String,
    pub to: String,
    pub source: String,
    pub ts: i64,
    pub status: String,
    pub ver: u64,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

impl Node {
    pub fn from_meta(m: &Value) -> Option<Node> {
        (m.get("t")?.as_str()? == "node").then(|| Node {
            key: s(m, "key"),
            kind: s(m, "kind"),
            title: s(m, "title"),
            body: s(m, "body"),
            source: s(m, "source"),
            status: s(m, "status"),
            ts: m.get("ts").and_then(Value::as_i64).unwrap_or(0),
            ver: m.get("ver").and_then(Value::as_u64).unwrap_or(0),
        })
    }
    pub fn to_meta(&self, emb: &str) -> Value {
        json!({"t":"node","key":self.key,"kind":self.kind,"title":self.title,"body":self.body,
               "source":self.source,"status":self.status,"ts":self.ts,"ver":self.ver,"emb":emb})
    }
    /// Same content ignoring the store-assigned version (idempotent re-ingest).
    pub fn same_content(&self, o: &Node) -> bool {
        (
            &self.key,
            &self.kind,
            &self.title,
            &self.body,
            &self.source,
            &self.status,
            self.ts,
        ) == (
            &o.key, &o.kind, &o.title, &o.body, &o.source, &o.status, o.ts,
        )
    }
}

impl Edge {
    pub fn from_meta(m: &Value) -> Option<Edge> {
        (m.get("t")?.as_str()? == "edge").then(|| Edge {
            rel: s(m, "rel"),
            from: s(m, "from"),
            to: s(m, "to"),
            source: s(m, "source"),
            ts: m.get("ts").and_then(Value::as_i64).unwrap_or(0),
            status: s(m, "status"),
            ver: m.get("ver").and_then(Value::as_u64).unwrap_or(0),
        })
    }
    pub fn to_meta(&self) -> Value {
        json!({"t":"edge","rel":self.rel,"from":self.from,"to":self.to,"source":self.source,
               "ts":self.ts,"status":self.status,"ver":self.ver})
    }
    pub fn other(&self, key: &str) -> &str {
        if self.from == key {
            &self.to
        } else {
            &self.from
        }
    }
}

pub fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// 53-bit ids so they survive a JSON number round trip. Nonzero.
pub fn id_of(text: &str) -> VectorId {
    VectorId((fnv(text) & ((1u64 << 53) - 1)).max(1))
}

const STOP: [&str; 30] = [
    "the", "and", "for", "did", "what", "why", "how", "are", "was", "were", "this", "that", "with",
    "from", "into", "show", "find", "has", "have", "does", "which", "who", "when", "where",
    "about", "there", "their", "these", "those", "not",
];

pub fn raw_words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

pub fn tokens(text: &str) -> Vec<String> {
    raw_words(text)
        .into_iter()
        .filter(|w| w.len() >= 3 && !STOP.contains(&w.as_str()))
        .map(|w| {
            if w.len() > 4 && w.ends_with('s') {
                w[..w.len() - 1].to_string()
            } else {
                w
            }
        })
        .collect()
}
