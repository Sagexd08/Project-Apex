use atlas_usearch::use_exact_scan;

#[test]
fn small_sets_are_always_exact() {
    assert!(use_exact_scan(10_000_000, 0, 512));
    assert!(use_exact_scan(10_000_000, 2_500, 512));
}

#[test]
fn full_and_large_sets_use_the_graph() {
    assert!(!use_exact_scan(1_000_000, 1_000_000, 512));
    assert!(!use_exact_scan(1_000_000, 500_000, 512));
    assert!(!use_exact_scan(1_183_514, 118_000, 512));
}

#[test]
fn crossover_follows_the_measured_s9_points() {
    assert!(use_exact_scan(60_000, 6_000, 512));
    assert!(use_exact_scan(290_000, 14_500, 512));
    assert!(use_exact_scan(1_183_514, 11_800, 512));
    assert!(!use_exact_scan(1_183_514, 118_000, 512));
}

#[test]
fn higher_ef_moves_the_crossover_up_and_never_overflows() {
    let n = 1_000_000;
    let boundary = |ef| (1..=n).take_while(|&a| use_exact_scan(n, a, ef)).count();
    assert!(boundary(1024) > boundary(256));
    assert!(use_exact_scan(usize::MAX, usize::MAX / 2, usize::MAX));
}
