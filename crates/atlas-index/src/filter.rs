use atlas_core::AtlasError;
use serde_json::Value;

pub const MAX_FILTER_DEPTH: usize = 16;
pub const MAX_FILTER_NODES: usize = 256;
pub const MAX_IN_VALUES: usize = 1024;
pub const MAX_PATH_SEGMENTS: usize = 8;
pub const MAX_FIELD_CHARS: usize = 128;

#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    Eq {
        path: Vec<String>,
        value: Value,
    },
    In {
        path: Vec<String>,
        values: Vec<Value>,
    },
    Range {
        path: Vec<String>,
        gt: Option<f64>,
        gte: Option<f64>,
        lt: Option<f64>,
        lte: Option<f64>,
    },
    And(Vec<Filter>),
    Or(Vec<Filter>),
    Not(Box<Filter>),
}

fn invalid(msg: &str) -> AtlasError {
    AtlasError::InvalidFilter(msg.to_string())
}

fn parse_path(v: &Value) -> Result<Vec<String>, AtlasError> {
    let s = v
        .as_str()
        .ok_or_else(|| invalid("field must be a string"))?;
    if s.is_empty() || s.chars().count() > MAX_FIELD_CHARS {
        return Err(invalid("field length out of range"));
    }
    let path: Vec<String> = s.split('.').map(str::to_string).collect();
    if path.len() > MAX_PATH_SEGMENTS || path.iter().any(String::is_empty) {
        return Err(invalid("field path malformed"));
    }
    Ok(path)
}

fn bound(obj: &serde_json::Map<String, Value>, key: &str) -> Result<Option<f64>, AtlasError> {
    match obj.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|f| f.is_finite())
            .map(Some)
            .ok_or_else(|| invalid("range bound must be a finite number")),
    }
}

fn pair(v: &Value) -> Result<(&Value, &Value), AtlasError> {
    match v.as_array().map(Vec::as_slice) {
        Some([a, b]) => Ok((a, b)),
        _ => Err(invalid("expected [field, value]")),
    }
}

fn parse_range(arg: &Value) -> Result<Filter, AtlasError> {
    let o = arg
        .as_object()
        .ok_or_else(|| invalid("range expects an object"))?;
    if o.keys()
        .any(|k| !["field", "gt", "gte", "lt", "lte"].contains(&k.as_str()))
    {
        return Err(invalid("unknown range key"));
    }
    let path = parse_path(o.get("field").ok_or_else(|| invalid("range needs field"))?)?;
    let (gt, gte, lt, lte) = (
        bound(o, "gt")?,
        bound(o, "gte")?,
        bound(o, "lt")?,
        bound(o, "lte")?,
    );
    if gt.is_none() && gte.is_none() && lt.is_none() && lte.is_none() {
        return Err(invalid("range needs at least one bound"));
    }
    Ok(Filter::Range {
        path,
        gt,
        gte,
        lt,
        lte,
    })
}

fn parse(v: &Value, depth: usize, nodes: &mut usize) -> Result<Filter, AtlasError> {
    if depth > MAX_FILTER_DEPTH {
        return Err(invalid("filter nested too deeply"));
    }
    *nodes += 1;
    if *nodes > MAX_FILTER_NODES {
        return Err(invalid("filter has too many nodes"));
    }
    let obj = v
        .as_object()
        .ok_or_else(|| invalid("filter node must be an object"))?;
    if obj.len() != 1 {
        return Err(invalid("filter node must have exactly one operator"));
    }
    let (op, arg) = obj.iter().next().ok_or_else(|| invalid("empty filter"))?;
    match op.as_str() {
        "eq" => {
            let (f, val) = pair(arg)?;
            Ok(Filter::Eq {
                path: parse_path(f)?,
                value: val.clone(),
            })
        }
        "in" => {
            let (f, vals) = pair(arg)?;
            let vals = vals
                .as_array()
                .ok_or_else(|| invalid("in expects an array of values"))?;
            if vals.is_empty() || vals.len() > MAX_IN_VALUES {
                return Err(invalid("in value count out of range"));
            }
            Ok(Filter::In {
                path: parse_path(f)?,
                values: vals.clone(),
            })
        }
        "range" => parse_range(arg),
        "and" | "or" => {
            let items = arg
                .as_array()
                .filter(|a| !a.is_empty())
                .ok_or_else(|| invalid("and/or expect a non-empty array"))?;
            let kids = items
                .iter()
                .map(|c| parse(c, depth + 1, nodes))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(if op == "and" {
                Filter::And(kids)
            } else {
                Filter::Or(kids)
            })
        }
        "not" => Ok(Filter::Not(Box::new(parse(arg, depth + 1, nodes)?))),
        _ => Err(invalid("unknown operator")),
    }
}

fn lookup<'a>(meta: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter()
        .try_fold(meta, |cur, seg| cur.as_object()?.get(seg))
}

fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) if a.is_number() && b.is_number() => x == y,
        _ => a == b,
    }
}

impl Filter {
    pub fn from_json(v: &Value) -> Result<Filter, AtlasError> {
        let mut nodes = 0;
        parse(v, 1, &mut nodes)
    }

    pub fn matches(&self, meta: &Value) -> bool {
        match self {
            Filter::Eq { path, value } => lookup(meta, path).is_some_and(|v| same(v, value)),
            Filter::In { path, values } => {
                lookup(meta, path).is_some_and(|v| values.iter().any(|x| same(v, x)))
            }
            Filter::Range {
                path,
                gt,
                gte,
                lt,
                lte,
            } => lookup(meta, path).and_then(Value::as_f64).is_some_and(|x| {
                gt.is_none_or(|b| x > b)
                    && gte.is_none_or(|b| x >= b)
                    && lt.is_none_or(|b| x < b)
                    && lte.is_none_or(|b| x <= b)
            }),
            Filter::And(k) => k.iter().all(|f| f.matches(meta)),
            Filter::Or(k) => k.iter().any(|f| f.matches(meta)),
            Filter::Not(f) => !f.matches(meta),
        }
    }
}
