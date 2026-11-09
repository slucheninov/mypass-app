// Test-only support code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use mypass_core::merge::merge;
use mypass_kdbx::{Database, DeletedObject, Entry, Group, Timestamp};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;
use uuid::Uuid;

fn database(rows: &[(u8, i64, u8)], deletes: &[(u8, i64)]) -> Database {
    let mut d = Database::new("test");
    d.root = Group::new("root", Timestamp(0));
    d.root.uuid = Uuid::from_u128(100);
    let mut child = Group::new("child", Timestamp(0));
    child.uuid = Uuid::from_u128(101);
    let mut grandchild = Group::new("grandchild", Timestamp(0));
    grandchild.uuid = Uuid::from_u128(102);
    for &(id, time, parent) in rows {
        let mut e = Entry::new(Timestamp(time));
        e.uuid = Uuid::from_u128(u128::from(id));
        e.set_field("Password", &time.to_string(), true);
        match parent {
            0 => d.root.entries.push(e),
            1 => child.entries.push(e),
            _ => grandchild.entries.push(e),
        }
    }
    child.groups.push(grandchild);
    d.root.groups.push(child);
    d.deleted_objects = deletes
        .iter()
        .map(|&(id, time)| DeletedObject {
            uuid: Uuid::from_u128(u128::from(id)),
            deleted_at: Timestamp(time),
        })
        .collect();
    d
}
fn entries(d: &Database) -> Vec<&Entry> {
    let mut out = vec![];
    let mut stack = vec![&d.root];
    while let Some(g) = stack.pop() {
        out.extend(&g.entries);
        stack.extend(&g.groups);
    }
    out
}
fn cases() -> impl Strategy<Value = (Database, Database)> {
    (
        prop::collection::vec((0u8..8, 0i64..10, 0u8..3), 0..=6),
        prop::collection::vec((0u8..8, 0i64..10, 0u8..3), 0..=6),
        prop::collection::vec((0u8..8, 0i64..10), 0..=8),
        prop::collection::vec((0u8..8, 0i64..10), 0..=8),
    )
        .prop_map(|(a, b, c, d)| (database(&a, &c), database(&b, &d)))
}
proptest! {
    #![proptest_config(ProptestConfig { cases:256, rng_seed:RngSeed::Fixed(0x1616), failure_persistence:None, ..ProptestConfig::default() })]
    #[test]
    fn merge_never_loses_a_live_entry((l,r) in cases()) {
        let m=merge(&l,&r); let out=entries(&m);
        for e in entries(&l).into_iter().chain(entries(&r)) {
            if !m.deleted_objects.iter().any(|d|d.uuid==e.uuid){prop_assert_eq!(out.iter().filter(|o|o.uuid==e.uuid).count(),1);}
        }
    }
    #[test]
    fn merged_deleted_objects_never_resurrect_newer_deletes((l,r) in cases()) {
        let m=merge(&l,&r);
        // Only first occurrences count, matching the documented malformed-input rule.
        let mut versions=std::collections::HashMap::new();
        for side in [&l,&r] {let mut seen=std::collections::HashSet::new();for e in entries(side){if seen.insert(e.uuid){versions.entry(e.uuid).and_modify(|t:&mut Timestamp|*t=(*t).max(e.modified)).or_insert(e.modified);}}}
        for d in l.deleted_objects.iter().chain(&r.deleted_objects) {
            if versions.get(&d.uuid).is_none_or(|t|*t<=d.deleted_at){prop_assert!(m.deleted_objects.iter().any(|o|o.uuid==d.uuid && o.deleted_at>=d.deleted_at));}
        }
    }
    #[test]
    fn no_duplicate_uuids_in_output((l,r) in cases()) {
        let m=merge(&l,&r); let mut seen=std::collections::HashSet::new();
        let mut stack=vec![&m.root];while let Some(g)=stack.pop(){prop_assert!(seen.insert(g.uuid));for e in &g.entries{prop_assert!(seen.insert(e.uuid));}stack.extend(&g.groups);}
    }
}
