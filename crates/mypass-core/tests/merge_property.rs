// Test-only support code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use mypass_core::merge::merge;
use mypass_kdbx::{CustomDataItem, Database, DeletedObject, Entry, Group, Timestamp};
use proptest::prelude::*;
use proptest::test_runner::{RngSeed, TestCaseResult};
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

type Row = (u8, i64, i64, u8, Vec<i64>);
type GroupRow = (bool, i64, i64, u8, u8, u8);

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
    // Independent per-side permutations permit any group to parent any other,
    // while earlier-only parents guarantee each input is acyclic.
    let mut shapes: Vec<_> = shapes.into_iter().enumerate().collect();
    shapes.sort_by_key(|(i, row)| (row.5, *i));
    let mut order = vec![];
    for (i, (present, modified, location, shape, value, _)) in shapes {
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
        let choices: Vec<_> = groups.keys().copied().collect();
        let parent = choices[usize::from(shape) % choices.len()];
        parents.insert(id, parent);
        order.push(id);
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
    for id in order.into_iter().rev() {
        let parent = parents[&id];
        let g = groups.remove(&id).unwrap();
        groups.get_mut(&parent).unwrap().groups.push(g);
    }
    d.root = groups.remove(&100).unwrap();
    d.deleted_objects = deletes
        .into_iter()
        .map(|(id, time)| DeletedObject {
            uuid: Uuid::from_u128(if id == 13 {
                100
            } else if id < 8 {
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
                    0u8..6,
                    prop::collection::vec(0i64..10, 0..=3),
                ),
                0..=6,
            ),
            prop::collection::vec(
                (
                    any::<bool>(),
                    0i64..10,
                    0i64..10,
                    any::<u8>(),
                    0u8..8,
                    any::<u8>(),
                ),
                5,
            ),
            prop::collection::vec((0u8..14, 0i64..10), 0..=8),
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

// A separate plain-map oracle: first choose clocks/parents, then identify
// cycle membership by closing each item's ancestor path. Only the minimum UUID
// of each cycle loses its parent. Descendants outside the cycle retain theirs.
fn locations(d: &Database) -> BTreeMap<Uuid, (Uuid, Timestamp, bool)> {
    let mut out = BTreeMap::new();
    let mut stack = vec![&d.root];
    while let Some(g) = stack.pop() {
        for child in &g.groups {
            out.insert(child.uuid, (g.uuid, child.location_changed, true));
            stack.push(child);
        }
        for e in &g.entries {
            out.insert(e.uuid, (g.uuid, e.location_changed, false));
        }
    }
    out
}

fn check_locations(l: &Database, r: &Database) -> TestCaseResult {
    check_survival(l, r)?;
    let root = l.root.uuid;
    let mut modified = BTreeMap::<Uuid, Timestamp>::new();
    let mut deletes = BTreeMap::<Uuid, Timestamp>::new();
    for (id, t) in items(l).into_iter().chain(items(r)) {
        modified
            .entry(id)
            .and_modify(|v| *v = (*v).max(t))
            .or_insert(t);
    }
    for d in l.deleted_objects.iter().chain(&r.deleted_objects) {
        deletes
            .entry(d.uuid)
            .and_modify(|v| *v = (*v).max(d.deleted_at))
            .or_insert(d.deleted_at);
    }
    let live: HashSet<_> = modified
        .iter()
        .filter_map(|(&id, t)| (id == root || deletes.get(&id).is_none_or(|d| t > d)).then_some(id))
        .collect();
    let mut expected = locations(r);
    for (id, value) in locations(l) {
        if expected.get(&id).is_none_or(|r| value.1 >= r.1) {
            expected.insert(id, value);
        }
    }
    expected.retain(|id, _| live.contains(id));
    for (parent, _, _) in expected.values_mut() {
        if !live.contains(parent) {
            *parent = root;
        }
    }
    let mut cuts = HashSet::new();
    for (&id, &(_, _, group)) in &expected {
        if !group {
            continue;
        }
        let mut path = vec![];
        let mut cursor = id;
        while cursor != root {
            if let Some(start) = path.iter().position(|p| *p == cursor) {
                cuts.insert(*path[start..].iter().min().unwrap());
                break;
            }
            path.push(cursor);
            cursor = expected[&cursor].0;
        }
    }
    for id in cuts {
        expected.get_mut(&id).unwrap().0 = root;
    }
    let actual = locations(&merge(l, r));
    prop_assert_eq!(actual, expected);
    Ok(())
}

fn tie_clocks(mut d: Database) -> Database {
    let mut stack = vec![&mut d.root];
    while let Some(g) = stack.pop() {
        g.modified.0 /= 2;
        g.location_changed.0 /= 2;
        for e in &mut g.entries {
            e.modified.0 /= 2;
            e.location_changed.0 /= 2;
        }
        stack.extend(&mut g.groups);
    }
    for t in &mut d.deleted_objects {
        t.deleted_at.0 /= 2;
    }
    d
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 1024, rng_seed: RngSeed::Fixed(0x1616), failure_persistence: None, ..ProptestConfig::default() })]
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
    fn surviving_locations_follow_move_clocks((l,r) in cases()) {
        check_locations(&l, &r)?;
        check_locations(&r, &l)?;
    }
    #[test]
    fn tied_locations_favour_local((l,r) in cases()) {
        check_locations(&tie_clocks(l), &tie_clocks(r))?;
    }
    #[test]
    fn merge_is_symmetric_up_to_ties((l,r) in cases()) {
        check_symmetric(&l, &r)?;
    }
}
