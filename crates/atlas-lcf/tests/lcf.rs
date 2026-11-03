use atlas_core::{AuthContext, Config, EffectiveScope, KeyScope, PermSet, Principal, TenantId};
use atlas_engine::Engine;
use atlas_lcf::{cnr::Opts, seed::seed_demo, Edge, Lcf, LcfError, LcfState, Node};
use std::path::PathBuf;

fn sc(t: &str) -> EffectiveScope {
    let t = TenantId::parse(t).unwrap();
    AuthContext::for_test(
        Principal {
            tenant: t.clone(),
            perms: PermSet::ALL,
        },
        KeyScope {
            tenant: t,
            allowed: PermSet::ALL,
        },
    )
    .effective()
}

fn dir(n: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("atlas-lcf-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn open(d: &PathBuf) -> Engine {
    Engine::open(d, Config::new(atlas_lcf::model::DIM)).unwrap()
}

fn node(key: &str, kind: &str) -> Node {
    Node {
        key: key.into(),
        kind: kind.into(),
        title: format!("title {key}"),
        body: "body".into(),
        source: "test".into(),
        status: "asserted".into(),
        ts: 1,
        ver: 0,
    }
}

fn edge(from: &str, rel: &str, to: &str) -> Edge {
    Edge {
        rel: rel.into(),
        from: from.into(),
        to: to.into(),
        source: "test".into(),
        ts: 1,
        status: "asserted".into(),
        ver: 0,
    }
}

fn keys(v: &serde_json::Value) -> Vec<String> {
    v["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["key"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn creates_nodes_and_edges_and_reads_neighbours() {
    let d = dir("basic");
    let e = open(&d);
    let st = LcfState::default();
    let (l, s) = (Lcf::new(&e, &st), sc("t1"));
    l.put_node(&s, &node("a", "decision")).unwrap();
    l.put_node(&s, &node("b", "experiment")).unwrap();
    l.put_edge(&s, &edge("a", "DERIVED_FROM", "b")).unwrap();
    let (adj, trunc) = l.adjacency(&s, "a").unwrap();
    assert_eq!(adj.len(), 1);
    assert!(!trunc);
    assert_eq!(adj[0].other("a"), "b");
    assert_eq!(l.get_node(&s, "a").unwrap().unwrap().kind, "decision");
}

#[test]
fn rejects_invalid_input_and_dangling_edges() {
    let d = dir("invalid");
    let e = open(&d);
    let st = LcfState::default();
    let (l, s) = (Lcf::new(&e, &st), sc("t1"));
    assert!(matches!(
        l.put_node(&s, &node("a", "nonsense")),
        Err(LcfError::Invalid(_))
    ));
    let mut n = node("a", "fact");
    n.source = String::new();
    assert!(matches!(l.put_node(&s, &n), Err(LcfError::Invalid(_))));
    l.put_node(&s, &node("a", "fact")).unwrap();
    assert!(matches!(
        l.put_edge(&s, &edge("a", "LIKES", "a")),
        Err(LcfError::Invalid(_))
    ));
    assert!(matches!(
        l.put_edge(&s, &edge("a", "SUPPORTS", "ghost")),
        Err(LcfError::NotFound(_))
    ));
}

#[test]
fn same_node_is_idempotent_but_changed_content_conflicts() {
    let d = dir("idem");
    let e = open(&d);
    let st = LcfState::default();
    let (l, s) = (Lcf::new(&e, &st), sc("t1"));
    let n = node("a", "fact");
    let s1 = l.put_node(&s, &n).unwrap();
    assert_eq!(l.put_node(&s, &n).unwrap(), s1);
    let mut changed = n.clone();
    changed.body = "different".into();
    assert!(matches!(
        l.put_node(&s, &changed),
        Err(LcfError::Conflict(_))
    ));
}

#[test]
fn supersede_keeps_old_record_and_marks_lifecycle() {
    let d = dir("supersede");
    let e = open(&d);
    let st = LcfState::default();
    let (l, s) = (Lcf::new(&e, &st), sc("t1"));
    l.put_node(&s, &node("v1", "fact")).unwrap();
    l.supersede(&s, "v1", &node("v2", "fact")).unwrap();
    assert!(l.get_node(&s, "v1").unwrap().is_some());
    let (adj, _) = l.adjacency(&s, "v1").unwrap();
    assert_eq!(
        Lcf::lifecycle(&adj, "v1"),
        ("superseded", Some("v2".into()))
    );
    let (adj2, _) = l.adjacency(&s, "v2").unwrap();
    assert_eq!(Lcf::lifecycle(&adj2, "v2").0, "current");
}

#[test]
fn knowledge_survives_restart() {
    let d = dir("restart");
    {
        let e = open(&d);
        let st = LcfState::default();
        seed_demo(&Lcf::new(&e, &st), &sc("t1")).unwrap();
    }
    let e = open(&d);
    let st = LcfState::default();
    let (l, s) = (Lcf::new(&e, &st), sc("t1"));
    assert!(l.get_node(&s, "dec-eventlog").unwrap().is_some());
    let r = l
        .query(
            &s,
            "Why did we adopt an append-only event log?",
            Opts::default(),
        )
        .unwrap();
    assert!(keys(&r).contains(&"dec-eventlog".to_string()));
    // seeding again after restart is a no-op, not a conflict
    seed_demo(&l, &s).unwrap();
}

#[test]
fn why_query_returns_connected_chain_with_provenance_and_trace() {
    let d = dir("why");
    let e = open(&d);
    let st = LcfState::default();
    let (l, s) = (Lcf::new(&e, &st), sc("t1"));
    seed_demo(&l, &s).unwrap();
    let r = l
        .query(
            &s,
            "Why did we adopt an append-only event log?",
            Opts::default(),
        )
        .unwrap();
    let k = keys(&r);
    for want in [
        "dec-eventlog",
        "obs-lock-contention",
        "exp-pg-bench",
        "req-orders-latency",
    ] {
        assert!(k.contains(&want.to_string()), "missing {want} in {k:?}");
    }
    assert!(!r["trace"].as_array().unwrap().is_empty());
    assert!(r["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|n| !n["source"].as_str().unwrap().is_empty()));
    assert!(r["stats"]["nodes"].as_u64().unwrap() <= 40);
    // a multi-hop node carries its path from the seed
    let ex = r["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["key"] == "exp-pg-bench")
        .unwrap();
    assert!(ex["depth"].as_u64().unwrap() >= 1);
    assert!(!ex["path_from_seed"].as_array().unwrap().is_empty());
}

#[test]
fn superseded_question_surfaces_lifecycle() {
    let d = dir("hist");
    let e = open(&d);
    let st = LcfState::default();
    let (l, s) = (Lcf::new(&e, &st), sc("t1"));
    seed_demo(&l, &s).unwrap();
    let r = l
        .query(
            &s,
            "Checkpoint interval is 250 thousand events, what did it replace?",
            Opts::default(),
        )
        .unwrap();
    let old = r["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["key"] == "fact-ckpt-1m")
        .expect("old fact reached");
    assert_eq!(old["lifecycle"], "superseded");
    assert_eq!(old["superseded_by"], "fact-ckpt-250k");
}

#[test]
fn evidence_question_reaches_evidence_node() {
    let d = dir("evid");
    let e = open(&d);
    let st = LcfState::default();
    let (l, s) = (Lcf::new(&e, &st), sc("t1"));
    seed_demo(&l, &s).unwrap();
    let r = l
        .query(
            &s,
            "Find the evidence supporting the lock contention observation",
            Opts::default(),
        )
        .unwrap();
    assert!(
        keys(&r).contains(&"evd-bench-log".to_string()),
        "{:?}",
        keys(&r)
    );
}

#[test]
fn bounds_are_enforced_and_reported() {
    let d = dir("bounds");
    let e = open(&d);
    let st = LcfState::default();
    let (l, s) = (Lcf::new(&e, &st), sc("t1"));
    seed_demo(&l, &s).unwrap();
    let o = Opts {
        max_hops: 4,
        max_nodes: 3,
        seeds: 1,
        as_of: None,
    };
    let r = l
        .query(&s, "Why did we adopt an append-only event log?", o)
        .unwrap();
    assert!(r["stats"]["nodes"].as_u64().unwrap() <= 3);
    assert_eq!(r["truncated"], true);
    let r0 = l
        .query(
            &s,
            "Why did we adopt an append-only event log?",
            Opts {
                max_hops: 0,
                max_nodes: 40,
                seeds: 1,
                as_of: None,
            },
        )
        .unwrap();
    assert_eq!(r0["stats"]["nodes"], 1);
}

#[test]
fn nothing_matches_is_reported_not_invented() {
    let d = dir("empty");
    let e = open(&d);
    let st = LcfState::default();
    let (l, s) = (Lcf::new(&e, &st), sc("t1"));
    let r = l.query(&s, "anything at all", Opts::default()).unwrap();
    assert_eq!(r["nodes"].as_array().unwrap().len(), 0);
    assert!(r["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w.as_str().unwrap().starts_with("NO_SEED")));
}

#[test]
fn tenants_cannot_see_each_others_knowledge() {
    let d = dir("tenant");
    let e = open(&d);
    let st = LcfState::default();
    let l = Lcf::new(&e, &st);
    seed_demo(&l, &sc("t1")).unwrap();
    let other = sc("t2");
    assert!(l.get_node(&other, "dec-eventlog").unwrap().is_none());
    let r = l
        .query(
            &other,
            "Why did we adopt an append-only event log?",
            Opts::default(),
        )
        .unwrap();
    assert_eq!(r["nodes"].as_array().unwrap().len(), 0);
    assert!(matches!(
        l.put_edge(&other, &edge("dec-eventlog", "SUPPORTS", "exp-pg-bench")),
        Err(LcfError::NotFound(_))
    ));
}

#[test]
fn unrelated_query_on_populated_db_matches_nothing() {
    let d = dir("unrelated");
    let e = open(&d);
    let st = LcfState::default();
    let (l, s) = (Lcf::new(&e, &st), sc("t1"));
    seed_demo(&l, &s).unwrap();
    for q in ["zzzz qqqq", "quantum banana", "xyzzy plugh"] {
        let r = l.query(&s, q, Opts::default()).unwrap();
        assert_eq!(
            r["nodes"].as_array().unwrap().len(),
            0,
            "query {q:?} invented matches"
        );
    }
}

#[test]
fn pinned_read_ignores_later_writes_and_version_survives_restart() {
    let d = dir("pin");
    let (v_before, v_after);
    {
        let e = open(&d);
        let st = LcfState::default();
        let (l, s) = (Lcf::new(&e, &st), sc("t1"));
        l.put_node(&s, &node("a", "decision")).unwrap();
        l.put_node(&s, &node("b", "experiment")).unwrap();
        l.put_edge(&s, &edge("a", "DERIVED_FROM", "b")).unwrap();
        v_before = l.current_version(&s).unwrap();
        l.put_node(&s, &node("c", "observation")).unwrap();
        l.put_edge(&s, &edge("a", "CAUSED_BY", "c")).unwrap();
        v_after = l.current_version(&s).unwrap();
        assert!(v_after > v_before);
        // a pinned read at v_before sees neither the new node nor the new edge
        assert!(l.get_node_at(&s, "c", Some(v_before)).unwrap().is_none());
        assert!(l.get_node_at(&s, "c", None).unwrap().is_some());
        assert_eq!(l.adjacency_at(&s, "a", Some(v_before)).unwrap().0.len(), 1);
        assert_eq!(l.adjacency_at(&s, "a", None).unwrap().0.len(), 2);
    }
    // restart: versions are rebuilt from durable records, pinned reads still agree
    let e = open(&d);
    let st = LcfState::default();
    let (l, s) = (Lcf::new(&e, &st), sc("t1"));
    assert_eq!(l.current_version(&s).unwrap(), v_after);
    assert!(l.get_node_at(&s, "c", Some(v_before)).unwrap().is_none());
    let r = l
        .query(
            &s,
            "title a",
            Opts {
                max_hops: 3,
                max_nodes: 40,
                seeds: 1,
                as_of: Some(v_before),
            },
        )
        .unwrap();
    assert_eq!(r["read_version"], v_before);
    assert!(
        !keys(&r).contains(&"c".to_string()),
        "pinned query leaked a later node"
    );
    let latest = l
        .query(
            &s,
            "title a",
            Opts {
                max_hops: 3,
                max_nodes: 40,
                seeds: 1,
                as_of: None,
            },
        )
        .unwrap();
    assert!(keys(&latest).contains(&"c".to_string()));
}

#[test]
fn ingest_is_idempotent_reports_conflicts_and_resolves_later_targets() {
    use atlas_lcf::ingest::Doc;
    let d = dir("ingest");
    let e = open(&d);
    let st = LcfState::default();
    let (l, s) = (Lcf::new(&e, &st), sc("t1"));
    let a = Doc {
        path: "a.md".into(),
        text: "---\nid: a\nkind: decision\nderived_from: b\n---\n# Decision A\nbecause of B".into(),
    };
    let r1 = l.ingest(&s, &[a]).unwrap();
    assert_eq!(r1["nodes_written"], 1);
    assert_eq!(
        r1["unresolved"].as_array().unwrap().len(),
        1,
        "edge to missing b must be reported"
    );
    let b = Doc {
        path: "b.md".into(),
        text: "---\nid: b\nkind: experiment\n---\n# Experiment B".into(),
    };
    let a2 = Doc {
        path: "a.md".into(),
        text: "---\nid: a\nkind: decision\nderived_from: b\n---\n# Decision A\nbecause of B".into(),
    };
    let r2 = l.ingest(&s, &[b.clone(), a2]).unwrap();
    assert_eq!(r2["nodes_written"], 1);
    assert_eq!(r2["nodes_unchanged"], 1);
    assert_eq!(
        r2["relationships_written"], 1,
        "edge resolves once the target exists"
    );
    assert!(r2["unresolved"].as_array().unwrap().is_empty());
    let r3 = l.ingest(&s, &[b]).unwrap();
    assert_eq!(
        (r3["nodes_written"].as_u64(), r3["nodes_unchanged"].as_u64()),
        (Some(0), Some(1))
    );
    let changed = Doc {
        path: "a.md".into(),
        text: "---\nid: a\nkind: decision\n---\n# Decision A\nreworded".into(),
    };
    let r4 = l.ingest(&s, &[changed]).unwrap();
    assert_eq!(r4["conflicts"].as_array().unwrap().len(), 1);
    let src = l.get_node(&s, "a").unwrap().unwrap().source;
    assert!(
        src.starts_with("file:a.md#sha256:") && src.len() == "file:a.md#sha256:".len() + 64,
        "{src}"
    );
}

#[test]
fn http_embedder_talks_to_an_openai_style_server_and_fails_loudly_when_down() {
    use atlas_lcf::embed::{Embedder, HttpEmbedder};
    use std::io::{Read, Write};
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        if let Ok((mut c, _)) = l.accept() {
            let mut buf = [0u8; 4096];
            let _ = c.read(&mut buf);
            let body = r#"{"data":[{"embedding":[3.0,4.0,0.0,0.0]}]}"#;
            let _ = write!(
                c,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
        }
    });
    let h = HttpEmbedder::new(&format!("http://127.0.0.1:{port}/v1/embeddings"), "m", 4).unwrap();
    let v = h.embed("hello").unwrap();
    assert!(
        (v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6,
        "normalized: {v:?}"
    );
    assert!(
        h.embed("again").is_err(),
        "server is gone: must error, not fall back"
    );
    assert!(HttpEmbedder::new("https://x/y", "m", 4).is_err());
}

#[test]
fn concurrent_writers_keep_pinned_reads_consistent() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let d = dir("conc");
    let e = open(&d);
    let st = LcfState::default();
    let s = sc("t1");
    let l = Lcf::new(&e, &st);
    let stop = AtomicBool::new(false);
    let checked = std::thread::scope(|scope| {
        let (l, s, stop) = (&l, &s, &stop);
        let reader = scope.spawn(move || {
            let mut checked = 0u32;
            while !stop.load(Ordering::SeqCst) {
                let v = l.current_version(s).unwrap();
                for w in 0..6 {
                    for i in (0..40).step_by(7) {
                        let a = format!("w{w}-a{i}");
                        for e in l.adjacency_at(s, &a, Some(v)).unwrap().0 {
                            for k in [&e.from, &e.to] {
                                assert!(
                                    l.get_node_at(s, k, Some(v)).unwrap().is_some(),
                                    "edge visible at v{v} but endpoint {k} is not"
                                );
                            }
                            checked += 1;
                        }
                    }
                }
            }
            checked
        });
        let writers: Vec<_> = (0..6)
            .map(|w| {
                scope.spawn(move || {
                    for i in 0..40 {
                        let (a, b) = (format!("w{w}-a{i}"), format!("w{w}-b{i}"));
                        l.put_node(s, &node(&a, "fact")).unwrap();
                        l.put_node(s, &node(&b, "fact")).unwrap();
                        l.put_edge(s, &edge(&a, "SUPPORTS", &b)).unwrap();
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        stop.store(true, Ordering::SeqCst);
        reader.join().unwrap()
    });
    let _ = checked;
    let v = l.current_version(&s).unwrap();
    assert_eq!(
        v,
        6 * 40 * 3,
        "every write got exactly one version, no gaps"
    );
}
