// Test-only support code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use mypass_core::merge::merge;
use mypass_kdbx::{CustomDataItem, Database, DeletedObject, Entry, Group, Timestamp};
use proptest::prelude::*;
use proptest::test_runner::{RngSeed, TestCaseResult};
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

type Row = (u8, i64, i64, u8, Vec<i64>);
type GroupRow = (bool, i64, i64, u8, u8);

fn database(rows: Vec<Row>, shapes: Vec<GroupRow>, deletes: Vec<(u8, i64)>, side: i64) -> Database {
    // Disjoint even/odd clocks across sides make symmetry's no-ties precondition
    // constructive, including history clocks. UUID pools are disjoint by type.
    let clock = |t| Timestamp(t * 2 + side);
    let mut d = Database::new("test");
    d.root = Group::new("root", clock(0));
    d.root.uuid = Uuid::from_u128(100);
    let mut groups = BTreeMap::new();
    groups.insert(100u8, d.root.clone());
    let mut parents = BTreeMap::new();
    for (i, (present, modified, location, shape, value)) in shapes.into_iter().enumerate() {
        if !present {
            continue;
        }
        let id = 101 + i as u8;
        let mut g = Group::new(&value.to_string(), clock(modified));
        g.uuid = Uuid::from_u128(u128::from(id));
        g.location_changed = clock(location);
        g.icon_id = u32::from(value);
        g.custom_data.push(CustomDataItem {
            key: format!("key{}", value % 2),
            value: value.to_string(),
            last_modified: Some(clock(modified)),
        });
        // Only group 101 can be a nested parent: depth is at most three.
        let parent = if id > 101 && shape % 2 == 1 && groups.contains_key(&101) {
            101
        } else {
            100
        };
        parents.insert(id, parent);
        groups.insert(id, g);
    }
    // Normalize duplicate generated rows before constructing a valid input tree.
    let rows: BTreeMap<_, _> = rows.into_iter().map(|r| (r.0, r)).collect();
    for (id, modified, location, parent, history) in rows.into_values() {
        let mut e = Entry::new(clock(modified));
        e.uuid = Uuid::from_u128(u128::from(id));
        e.location_changed = clock(location);
        e.set_field("Password", &modified.to_string(), true);
        for t in history {
            let mut h = Entry::new(clock(t));
            h.uuid = e.uuid;
            h.set_field("Password", &t.to_string(), true);
            e.history.push(h);
        }
        let parent = 100 + parent;
        let parent = if groups.contains_key(&parent) {
            parent
        } else {
            100
        };
        groups.get_mut(&parent).unwrap().entries.push(e);
    }
    for (id, parent) in parents.into_iter().rev() {
        let g = groups.remove(&id).unwrap();
        groups.get_mut(&parent).unwrap().groups.push(g);
    }
    d.root = groups.remove(&100).unwrap();
    d.deleted_objects = deletes
        .into_iter()
        .map(|(id, time)| DeletedObject {
            uuid: Uuid::from_u128(if id < 8 {
                u128::from(id)
            } else {
                101 + u128::from(id - 8)
            }),
            deleted_at: clock(time),
        })
        .collect();
    d
}

fn cases() -> impl Strategy<Value = (Database, Database)> {
    let side = || {
        (
            prop::collection::vec(
                (
                    0u8..8,
                    0i64..10,
                    0i64..10,
                    0u8..4,
                    prop::collection::vec(0i64..10, 0..=3),
                ),
                0..=6,
            ),
            prop::collection::vec((any::<bool>(), 0i64..10, 0i64..10, any::<u8>(), 0u8..8), 3),
            prop::collection::vec((0u8..11, 0i64..10), 0..=8),
        )
    };
    (side(), side()).prop_map(|((a, b, c), (d, e, f))| (database(a, b, c, 0), database(d, e, f, 1)))
}

fn items(d: &Database) -> Vec<(Uuid, Timestamp)> {
    let mut out = vec![];
    let mut stack = vec![&d.root];
    while let Some(g) = stack.pop() {
        out.push((g.uuid, g.modified));
        out.extend(g.entries.iter().map(|e| (e.uuid, e.modified)));
        stack.extend(&g.groups);
    }
    out
}

fn check_survival(l: &Database, r: &Database) -> TestCaseResult {
    // Independent oracle: plain maps over both inputs; no merge indexing/helpers,
    // and no reliance on output tombstones to decide which objects should live.
    let mut modified = HashMap::<Uuid, Timestamp>::new();
    let mut deleted = HashMap::<Uuid, Timestamp>::new();
    for side in [l, r] {
        for (id, time) in items(side) {
            modified
                .entry(id)
                .and_modify(|t| *t = (*t).max(time))
                .or_insert(time);
        }
        for d in &side.deleted_objects {
            deleted
                .entry(d.uuid)
                .and_modify(|t| *t = (*t).max(d.deleted_at))
                .or_insert(d.deleted_at);
        }
    }
    let m = merge(l, r);
    let mut counts = HashMap::<Uuid, usize>::new();
    for (id, _) in items(&m) {
        *counts.entry(id).or_default() += 1;
    }
    for d in &m.deleted_objects {
        prop_assert!(
            deleted.contains_key(&d.uuid),
            "invented tombstone {}",
            d.uuid
        );
    }
    for (&id, &time) in &modified {
        let live = id == l.root.uuid || deleted.get(&id).is_none_or(|d| time > *d);
        prop_assert_eq!(
            counts.get(&id).copied().unwrap_or(0),
            usize::from(live),
            "uuid {}",
            id
        );
        if live {
            prop_assert!(
                !m.deleted_objects.iter().any(|d| d.uuid == id),
                "live tombstoned {}",
                id
            );
        }
    }
    for id in deleted.keys() {
        if !modified.contains_key(id) {
            prop_assert!(!counts.contains_key(id));
        }
    }
    Ok(())
}

fn check_deletions(l: &Database, r: &Database) -> TestCaseResult {
    let m = merge(l, r);
    let mut versions = HashMap::<Uuid, Timestamp>::new();
    for (id, time) in items(l).into_iter().chain(items(r)) {
        versions
            .entry(id)
            .and_modify(|t| *t = (*t).max(time))
            .or_insert(time);
    }
    for d in l.deleted_objects.iter().chain(&r.deleted_objects) {
        if d.uuid != l.root.uuid && versions.get(&d.uuid).is_none_or(|t| *t <= d.deleted_at) {
            prop_assert!(m
                .deleted_objects
                .iter()
                .any(|o| o.uuid == d.uuid && o.deleted_at >= d.deleted_at));
        }
    }
    Ok(())
}

fn check_unique(l: &Database, r: &Database) -> TestCaseResult {
    let m = merge(l, r);
    let mut seen = HashSet::new();
    for (id, _) in items(&m) {
        prop_assert!(seen.insert(id));
    }
    Ok(())
}

fn check_symmetric(l: &Database, r: &Database) -> TestCaseResult {
    // Generator guarantees every cross-side clock comparison is tie-free.
    let a = merge(l, r);
    let b = merge(r, l);
    prop_assert_eq!(a.root, b.root);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, rng_seed: RngSeed::Fixed(0x1616), failure_persistence: None, ..ProptestConfig::default() })]
    #[test]
    fn surviving_items_are_not_tombstoned_and_tombstones_are_not_invented((l,r) in cases()) {
        check_survival(&l, &r)?;
    }
    #[test]
    fn merged_deleted_objects_never_resurrect_newer_deletes((l,r) in cases()) {
        check_deletions(&l, &r)?;
    }
    #[test]
    fn no_duplicate_uuids_in_output((l,r) in cases()) {
        check_unique(&l, &r)?;
    }
    #[test]
    fn merge_is_symmetric_up_to_ties((l,r) in cases()) {
        check_symmetric(&l, &r)?;
    }
}
