use atlas_core::VectorId;
use atlas_index::{AnnIndex, Filter, FlatIndex, MAX_FILTER_DEPTH, MAX_FILTER_NODES};
use serde_json::{json, Value};

fn f(v: Value) -> Filter {
    Filter::from_json(&v).unwrap()
}

fn nested_not(depth: usize) -> Value {
    let mut v = json!({"eq": ["a", 1]});
    for _ in 1..depth {
        v = json!({ "not": v });
    }
    v
}

#[test]
fn rejects_malformed_filters() {
    let bad = [
        json!([]),
        json!({}),
        json!({"eq": ["a"]}),
        json!({"eq": [1, 2]}),
        json!({"and": []}),
        json!({"or": {}}),
        json!({"range": {"field": "a"}}),
        json!({"range": {"field": "a", "gt": "x"}}),
        json!({"range": {"field": "a", "gt": 1, "bogus": 2}}),
        json!({"nope": 1}),
        json!({"eq": ["a", 1], "in": ["a", [1]]}),
        json!({"in": ["a", []]}),
        json!({"in": ["a", 3]}),
        json!({"eq": ["a..b", 1]}),
        json!({"eq": ["", 1]}),
        json!("eq"),
    ];
    for b in bad {
        let e = Filter::from_json(&b).unwrap_err();
        assert_eq!(e.code(), "INVALID_FILTER", "{b}");
    }
}

#[test]
fn depth_and_node_limits_are_enforced() {
    assert!(Filter::from_json(&nested_not(MAX_FILTER_DEPTH)).is_ok());
    assert!(Filter::from_json(&nested_not(MAX_FILTER_DEPTH + 1)).is_err());
    assert!(Filter::from_json(&nested_not(100)).is_err());
    let wide: Vec<Value> = (0..MAX_FILTER_NODES + 1)
        .map(|i| json!({"eq": ["a", i]}))
        .collect();
    assert!(Filter::from_json(&json!({"and": wide})).is_err());
    let ok: Vec<Value> = (0..MAX_FILTER_NODES - 1)
        .map(|i| json!({"eq": ["a", i]}))
        .collect();
    assert!(Filter::from_json(&json!({"or": ok})).is_ok());
}

#[test]
fn matching_semantics() {
    let m = json!({"a": 1, "s": "red", "o": {"k": true, "n": 2.5}});
    assert!(f(json!({"eq": ["a", 1.0]})).matches(&m));
    assert!(f(json!({"eq": ["s", "red"]})).matches(&m));
    assert!(!f(json!({"eq": ["s", 1]})).matches(&m));
    assert!(f(json!({"eq": ["o.k", true]})).matches(&m));
    assert!(!f(json!({"eq": ["missing", 1]})).matches(&m));
    assert!(f(json!({"not": {"eq": ["missing", 1]}})).matches(&m));
    assert!(f(json!({"in": ["s", ["blue", "red"]]})).matches(&m));
    assert!(!f(json!({"in": ["s", ["blue"]]})).matches(&m));
    assert!(f(json!({"range": {"field": "o.n", "gt": 2, "lte": 2.5}})).matches(&m));
    assert!(!f(json!({"range": {"field": "o.n", "gt": 2.5}})).matches(&m));
    assert!(!f(json!({"range": {"field": "s", "gt": 0}})).matches(&m));
    let nested =
        json!({"and": [{"eq": ["a", 1]}, {"or": [{"eq": ["s", "x"]}, {"eq": ["s", "red"]}]}]});
    assert!(f(nested).matches(&m));
    assert!(!f(json!({"eq": ["a", 1]})).matches(&Value::Null));
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn random_filter(r: &mut Lcg, depth: usize) -> Value {
    let colors = ["red", "green", "blue", "gold"];
    let pick = if depth >= 3 {
        r.next() % 3
    } else {
        r.next() % 6
    };
    match pick {
        0 => json!({"eq": ["c", colors[(r.next() % 4) as usize]]}),
        1 => {
            let lo = (r.next() % 50) as f64;
            let hi = 50.0 + (r.next() % 50) as f64;
            json!({"range": {"field": "n", "gte": lo, "lt": hi}})
        }
        2 => json!({"in": ["o.k", [r.next().is_multiple_of(2), false]]}),
        3 => json!({"and": [random_filter(r, depth + 1), random_filter(r, depth + 1)]}),
        4 => json!({"or": [random_filter(r, depth + 1), random_filter(r, depth + 1)]}),
        _ => json!({"not": random_filter(r, depth + 1)}),
    }
}

#[test]
fn flat_matching_ids_equals_naive_evaluation() {
    let colors = ["red", "green", "blue", "gold"];
    let mut r = Lcg(42);
    let mut ix = FlatIndex::new(2);
    for id in 0..600u64 {
        let md = json!({
            "c": colors[(r.next() % 4) as usize],
            "n": (r.next() % 100) as i64,
            "o": {"k": r.next().is_multiple_of(2)}
        });
        ix.insert(VectorId(id), &[id as f32, 1.0], md).unwrap();
    }
    let entries = ix.entries();
    for _ in 0..200 {
        let flt = f(random_filter(&mut r, 0));
        let mut got: Vec<u64> = ix.matching_ids(&flt).into_iter().map(|i| i.0).collect();
        let mut want: Vec<u64> = entries
            .iter()
            .filter(|(_, _, m)| flt.matches(m))
            .map(|(i, _, _)| i.0)
            .collect();
        got.sort();
        want.sort();
        assert_eq!(got, want);
    }
}
