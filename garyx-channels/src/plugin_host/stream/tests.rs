use super::*;

#[test]
fn generator_produces_process_unique_ids() {
    let g = StreamIdGenerator::new();
    let a = g.next();
    let b = g.next();
    let c = g.next();
    assert_ne!(a, b);
    assert_ne!(b, c);
    assert_ne!(a, c);
    assert!(a.as_str().starts_with("str_"));
}

#[test]
fn two_generators_never_collide_for_first_id() {
    // Different nonces mean IDs from different generator
    // instances don't collide even if their counters match.
    let g1 = StreamIdGenerator::with_nonce("aaaa0000");
    let g2 = StreamIdGenerator::with_nonce("bbbb0000");
    assert_ne!(g1.next(), g2.next());
}

#[test]
fn tombstone_registry_idempotent() {
    let reg = StreamRegistry::new();
    let id = StreamId::from("str_xyz_0");
    assert!(reg.tombstone(&id, TombstoneReason::Abandoned));
    assert!(!reg.tombstone(&id, TombstoneReason::Abandoned));
    assert!(reg.is_tombstoned(&id));
    assert!(!reg.is_tombstoned(&StreamId::from("str_other_0")));
}

#[test]
fn registry_tracks_size_for_diagnostics() {
    let reg = StreamRegistry::new();
    assert_eq!(reg.len(), 0);
    for i in 0..10u64 {
        reg.tombstone(
            &StreamId::from(format!("str_x_{i}")),
            TombstoneReason::Abandoned,
        );
    }
    assert_eq!(reg.len(), 10);
}

#[test]
fn generator_is_clone_and_shares_counter() {
    let g = StreamIdGenerator::new();
    let g2 = g.clone();
    let a = g.next();
    let b = g2.next();
    // Shared counter: they should not repeat the same suffix.
    let suffix_a = a.as_str().rsplit('_').next().unwrap();
    let suffix_b = b.as_str().rsplit('_').next().unwrap();
    assert_ne!(suffix_a, suffix_b);
}
