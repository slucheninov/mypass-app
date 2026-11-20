use super::*;
fn t(n: i64) -> Timestamp {
    Timestamp(1_700_000_000 + n)
}
fn make_entry(title: &str) -> Entry {
    let mut e = Entry::new(t(0));
    e.set_field("Title", title, false);
    e
}
fn make_db_entry() -> (Database, Uuid) {
    let mut db = Database::new("Root");
    let e = make_entry("v0");
    let id = e.uuid;
    db.add_entry(db.root_id(), e, t(1)).unwrap();
    (db, id)
}
#[test]
fn root_id_is_the_root_group_uuid() {
    let db = Database::new("Root");
    assert_eq!(db.root_id(), db.root.uuid);
}
#[test]
fn mark_accessed_sets_accessed_and_increments_usage_only() {
    let (mut db, id) = make_db_entry();
    let mut before = db.find_entry(id).unwrap().clone();
    assert!(db.mark_accessed(id, t(2)));
    before.accessed = t(2);
    before.usage_count = 1;
    assert_eq!(db.find_entry(id), Some(&before));
    db.find_entry_mut(id).unwrap().usage_count = u32::MAX;
    db.mark_accessed(id, t(3));
    assert_eq!(db.find_entry(id).unwrap().usage_count, u32::MAX);
}
#[test]
fn mark_accessed_missing_entry_returns_false() {
    assert!(!Database::new("Root").mark_accessed(Uuid::nil(), t(1)));
}
#[test]
fn update_snapshots_previous_version_into_history() {
    let (mut db, id) = make_db_entry();
    let before = db.find_entry(id).unwrap().clone();
    assert!(db
        .update_entry(id, t(2), |e| e.set_field("Title", "v1", false))
        .unwrap());
    let e = db.find_entry(id).unwrap();
    assert_eq!(e.modified, t(2));
    assert_eq!(e.history, vec![before]);
}
#[test]
fn no_op_update_records_nothing_and_keeps_modified() {
    let (mut db, id) = make_db_entry();
    let before = db.clone();
    assert!(!db
        .update_entry(id, t(2), |e| {
            e.modified = t(99);
            e.accessed = t(99);
            e.usage_count = 99;
        })
        .unwrap());
    assert_eq!(db, before);
}
#[test]
fn history_snapshot_has_no_nested_history_and_edit_cannot_replace_identity_or_history() {
    let (mut db, id) = make_db_entry();
    for n in 1..=2 {
        db.update_entry(id, t(n), |e| e.set_field("Title", &format!("v{n}"), false))
            .unwrap();
    }
    db.update_entry(id, t(3), |e| {
        e.uuid = Uuid::nil();
        e.history.clear();
        e.set_field("Title", "v3", false);
    })
    .unwrap();
    let e = db.find_entry(id).unwrap();
    assert_eq!(e.history.len(), 3);
    assert!(e.history.iter().all(|h| h.history.is_empty()));
    let before = db.clone();
    assert!(!db
        .update_entry(id, t(4), |e| {
            e.uuid = Uuid::nil();
            e.history.clear();
        })
        .unwrap());
    assert_eq!(db, before);
}
#[test]
fn history_max_items_three_keeps_newest_three() {
    let (mut db, id) = make_db_entry();
    db.meta.history_max_items = 3;
    for n in 1..=5 {
        db.update_entry(id, t(n), |e| e.set_field("Title", &format!("v{n}"), false))
            .unwrap();
    }
    assert_eq!(
        db.find_entry(id)
            .unwrap()
            .history
            .iter()
            .map(Entry::title)
            .collect::<Vec<_>>(),
        vec!["v2", "v3", "v4"]
    );
}
#[test]
fn history_max_items_zero_and_negative() {
    for (cap, want) in [(0, 0), (-1, 12)] {
        let (mut db, id) = make_db_entry();
        db.meta.history_max_items = cap;
        for n in 1..=12 {
            db.update_entry(id, t(n), |e| e.set_field("Title", &format!("v{n}"), false))
                .unwrap();
        }
        assert_eq!(db.find_entry(id).unwrap().history.len(), want);
    }
}
#[test]
fn history_size_boundaries_and_oldest_first() {
    // Title key = 5 bytes, value = 55: each snapshot is exactly 60.
    for (cap, edits, want) in [(100, 2, 1), (60, 1, 1), (59, 1, 0), (0, 1, 0), (-1, 2, 2)] {
        let (mut db, id) = make_db_entry();
        db.meta.history_max_size = cap;
        db.find_entry_mut(id)
            .unwrap()
            .set_field("Title", &"a".repeat(55), false);
        for n in 1..=edits {
            db.update_entry(id, t(n), |e| {
                e.set_field(
                    "Title",
                    &"b".repeat(54)
                        .chars()
                        .chain(std::iter::once(char::from(b'0' + n as u8)))
                        .collect::<String>(),
                    false,
                )
            })
            .unwrap();
        }
        let h = &db.find_entry(id).unwrap().history;
        assert_eq!(h.len(), want, "cap {cap}");
        if edits == 2 && cap == 100 {
            assert!(h[0].title().ends_with('1'));
        }
    }
    let (mut db, id) = make_db_entry();
    db.meta.history_max_size = 0;
    db.find_entry_mut(id).unwrap().fields.clear();
    db.update_entry(id, t(2), |e| e.icon_id = 1).unwrap();
    assert_eq!(db.find_entry(id).unwrap().history.len(), 1);
}
#[test]
fn attachment_bytes_and_names_count_toward_history_size() {
    for cap in [13, 14] {
        let (mut db, id) = make_db_entry();
        db.meta.history_max_size = cap;
        db.find_entry_mut(id).unwrap().attachments.push(Attachment {
            name: "abc".into(),
            data: vec![0; 4],
            protected: false,
        }); // Title+v0 = 7, name+data = 7.
        db.update_entry(id, t(2), |e| e.icon_id = 1).unwrap();
        assert_eq!(
            db.find_entry(id).unwrap().history.len(),
            usize::from(cap == 14)
        );
    }
}
#[test]
fn delete_moves_to_recycle_bin_and_creates_it_once() {
    let (mut db, id) = make_db_entry();
    let e = make_entry("second");
    let id2 = e.uuid;
    db.add_entry(db.root_id(), e, t(1)).unwrap();
    db.delete_entry(id, t(2)).unwrap();
    let bin = db.meta.recycle_bin_uuid.unwrap();
    db.delete_entry(id2, t(3)).unwrap();
    assert_eq!(db.meta.recycle_bin_uuid, Some(bin));
    assert_eq!(db.root.groups.len(), 1);
    assert_eq!(db.find_group(bin).unwrap().name, "Recycle Bin");
    assert_eq!(db.find_group(bin).unwrap().icon_id, 43);
    assert!(db.is_in_recycle_bin(id));
    assert!(db.is_in_recycle_bin(id2));
    assert!(db.deleted_objects.is_empty());
    assert_eq!(db.find_entry(id).unwrap().modified, t(0));
    assert_eq!(db.find_entry(id).unwrap().location_changed, t(2));
}
#[test]
fn dangling_recycle_bin_is_recreated() {
    let (mut db, id) = make_db_entry();
    db.meta.recycle_bin_uuid = Some(Uuid::nil());
    db.delete_entry(id, t(2)).unwrap();
    let bin = db.meta.recycle_bin_uuid.unwrap();
    assert_ne!(bin, Uuid::nil());
    assert_eq!(db.parent_of_entry(id), Some(bin));
}
#[test]
fn delete_in_bin_and_disabled_purge_with_now() {
    for enabled in [false, true] {
        let (mut db, id) = make_db_entry();
        db.meta.recycle_bin_enabled = enabled;
        if enabled {
            db.delete_entry(id, t(2)).unwrap();
        }
        db.delete_entry(id, t(3)).unwrap();
        assert!(db.find_entry(id).is_none());
        assert_eq!(
            db.deleted_objects,
            vec![DeletedObject {
                uuid: id,
                deleted_at: t(3)
            }]
        );
    }
}
#[test]
fn restore_moves_out_of_bin_and_move_preserves_content() {
    let (mut db, id) = make_db_entry();
    let before = db.find_entry(id).unwrap().clone();
    db.delete_entry(id, t(2)).unwrap();
    db.restore_entry(id, db.root_id(), t(3)).unwrap();
    let mut expected = before;
    expected.location_changed = t(3);
    // Content is untouched; only the location data changes (the source of the move is the bin).
    expected.previous_parent = db.meta.recycle_bin_uuid;
    assert_eq!(db.find_entry(id), Some(&expected));
    assert!(!db.is_in_recycle_bin(id));
}
#[test]
fn add_duplicate_and_missing_parent_rejected_atomically() {
    let (mut db, id) = make_db_entry();
    let before = db.clone();
    assert!(db
        .add_entry(db.root_id(), db.find_entry(id).unwrap().clone(), t(2))
        .is_err());
    assert!(db.add_entry(Uuid::nil(), make_entry("new"), t(2)).is_err());
    assert_eq!(db, before);
}
#[test]
fn move_group_into_descendant_and_root_deletion_rejected() {
    let mut db = Database::new("Root");
    let mut a = Group::new("a", t(0));
    let b = Group::new("b", t(0));
    let (aid, bid) = (a.uuid, b.uuid);
    a.groups.push(b);
    db.add_group(db.root_id(), a, t(1)).unwrap();
    let before = db.clone();
    assert!(db.move_group(aid, bid, t(2)).is_err());
    assert!(db.move_group(aid, aid, t(2)).is_err());
    assert!(db.delete_group(db.root_id(), t(2)).is_err());
    assert!(db.purge_group(db.root_id(), t(2)).is_err());
    assert_eq!(db, before);
}
#[test]
fn purge_group_tombstones_descendants_but_not_history_and_dedupes() {
    let mut db = Database::new("Root");
    let mut a = Group::new("a", t(0));
    let mut b = Group::new("b", t(0));
    let mut e = make_entry("e");
    let snapshot = make_entry("history");
    let hid = snapshot.uuid;
    e.history.push(snapshot);
    let (aid, bid, eid) = (a.uuid, b.uuid, e.uuid);
    b.entries.push(e);
    a.groups.push(b);
    db.add_group(db.root_id(), a, t(1)).unwrap();
    db.deleted_objects.push(DeletedObject {
        uuid: eid,
        deleted_at: t(0),
    });
    db.purge_group(aid, t(2)).unwrap();
    assert_eq!(db.deleted_objects.len(), 3);
    for id in [aid, bid] {
        assert!(db.deleted_objects.contains(&DeletedObject {
            uuid: id,
            deleted_at: t(2)
        }));
    }
    assert!(!db.deleted_objects.iter().any(|d| d.uuid == hid));
    let e = make_entry("again");
    let id = e.uuid;
    db.add_entry(db.root_id(), e.clone(), t(1)).unwrap();
    db.purge_entry(id, t(2)).unwrap();
    db.add_entry(db.root_id(), e, t(3)).unwrap();
    db.purge_entry(id, t(4)).unwrap();
    assert_eq!(
        db.deleted_objects.iter().filter(|d| d.uuid == id).count(),
        1
    );
}
#[test]
fn entries_excluding_prunes_subtrees_in_document_order() {
    let (mut db, id) = make_db_entry();
    let mut a = Group::new("a", t(0));
    let aid = a.uuid;
    a.entries.push(make_entry("a"));
    let mut b = Group::new("b", t(0));
    b.entries.push(make_entry("b"));
    a.groups.push(b);
    db.add_group(db.root_id(), a, t(1)).unwrap();
    assert_eq!(
        db.entries_excluding(&[])
            .iter()
            .map(|e| e.title())
            .collect::<Vec<_>>(),
        vec!["v0", "a", "b"]
    );
    assert_eq!(
        db.entries_excluding(&[aid])
            .iter()
            .map(|e| e.uuid)
            .collect::<Vec<_>>(),
        vec![id]
    );
    assert!(db.entries_excluding(&[db.root_id()]).is_empty());
}
#[test]
fn depth_limit_add_move_and_all_helpers_on_default_stack() {
    let mut db = Database::new("Root");
    let root = db.root_id();
    let mut parent = root;
    for _ in 1..93 {
        let g = Group::new("g", t(0));
        let id = g.uuid;
        db.add_group(parent, g, t(1)).unwrap();
        parent = id;
    }
    let before = db.clone();
    assert!(db
        .add_group(parent, Group::new("over", t(0)), t(1))
        .is_err());
    assert_eq!(db, before);
    let e = make_entry("leaf");
    let eid = e.uuid;
    db.add_entry(parent, e, t(1)).unwrap();
    assert_eq!(db.find_entry(eid).unwrap().title(), "leaf");
    db.mark_accessed(eid, t(2));
    db.update_entry(eid, t(3), |e| e.icon_id = 2).unwrap();
    assert_eq!(db.parent_of_entry(eid), Some(parent));
    assert_eq!(db.entries_excluding(&[]).len(), 1);
    let g = Group::new("moving", t(0));
    let gid = g.uuid;
    db.add_group(root, g, t(1)).unwrap();
    assert!(db.move_group(gid, parent, t(2)).is_err());
    let almost = db.root.groups[0].uuid;
    db.move_group(gid, root, t(2)).unwrap();
    db.move_entry(eid, gid, t(3)).unwrap();
    db.purge_group(almost, t(4)).unwrap();
    assert!(db.find_group(parent).is_none());
}

#[test]
fn mark_accessed_does_not_snapshot_history() {
    let (mut db, id) = make_db_entry();
    db.update_entry(id, t(2), |e| e.icon_id = 1).unwrap();
    let mut before = db.find_entry(id).unwrap().clone();
    db.mark_accessed(id, t(3));
    before.accessed = t(3);
    before.usage_count = 1;
    assert_eq!(db.find_entry(id), Some(&before));
}
#[test]
fn group_recycle_restore_move_and_bin_protection() {
    let mut db = Database::new("Root");
    let mut a = Group::new("a", t(0));
    let mut nested = Group::new("nested", t(0));
    let e = make_entry("inside");
    let eid = e.uuid;
    nested.entries.push(e);
    a.groups.push(nested);
    let aid = a.uuid;
    db.add_group(db.root_id(), a, t(1)).unwrap();
    db.delete_group(aid, t(2)).unwrap();
    let bin = db.meta.recycle_bin_uuid.unwrap();
    assert!(db.is_in_recycle_bin(eid));
    assert_eq!(db.find_group(aid).unwrap().modified, t(0));
    assert_eq!(db.find_group(aid).unwrap().location_changed, t(2));
    let before = db.clone();
    assert!(db.delete_group(bin, t(3)).is_err());
    assert!(db.purge_group(bin, t(3)).is_err());
    assert_eq!(db, before);
    db.move_group(aid, db.root_id(), t(3)).unwrap();
    assert!(!db.is_in_recycle_bin(eid));
    db.delete_group(aid, t(4)).unwrap();
    db.delete_group(aid, t(5)).unwrap();
    assert!(db.find_group(aid).is_none());
    assert_eq!(db.deleted_objects.len(), 3);
    let g = Group::new("disabled", t(0));
    let gid = g.uuid;
    db.add_group(db.root_id(), g, t(1)).unwrap();
    db.meta.recycle_bin_enabled = false;
    db.delete_group(gid, t(6)).unwrap();
    assert!(db.deleted_objects.contains(&DeletedObject {
        uuid: gid,
        deleted_at: t(6)
    }));
}
#[test]
fn missing_operations_and_subtree_duplicates_leave_database_unchanged() {
    let (mut db, id) = make_db_entry();
    let root = db.root_id();
    let before = db.clone();
    assert!(db.move_entry(id, Uuid::nil(), t(1)).is_err());
    assert!(db.move_entry(Uuid::nil(), root, t(1)).is_err());
    assert!(db.restore_entry(Uuid::nil(), root, t(1)).is_err());
    assert!(db
        .update_entry(Uuid::nil(), t(1), |_| panic!("must not run"))
        .is_err());
    assert!(db.delete_entry(Uuid::nil(), t(1)).is_err());
    assert!(db.purge_entry(Uuid::nil(), t(1)).is_err());
    assert!(db.move_group(Uuid::nil(), root, t(1)).is_err());
    assert!(db.move_group(root, root, t(1)).is_err());
    assert!(db.delete_group(Uuid::nil(), t(1)).is_err());
    assert!(db.purge_group(Uuid::nil(), t(1)).is_err());
    assert!(db
        .add_group(Uuid::nil(), Group::new("missing", t(0)), t(1))
        .is_err());
    let mut g = Group::new("duplicate", t(0));
    g.entries.push(db.find_entry(id).unwrap().clone());
    assert!(db.add_group(root, g, t(1)).is_err());
    let mut g = Group::new("internal duplicate", t(0));
    let child = Group::new("child", t(0));
    g.groups.push(child.clone());
    g.groups.push(child);
    assert!(db.add_group(root, g, t(1)).is_err());
    assert_eq!(db, before);
}
#[test]
fn move_subtree_exact_limit_and_one_over_are_atomic() {
    let mut db = Database::new("Root");
    let root = db.root_id();
    let mut parent = root;
    for _ in 1..91 {
        let g = Group::new("ancestor", t(0));
        let id = g.uuid;
        db.add_group(parent, g, t(1)).unwrap();
        parent = id;
    }
    let mut subtree = Group::new("subtree", t(0));
    let leaf = Group::new("leaf", t(0));
    let leafid = leaf.uuid;
    subtree.groups.push(leaf);
    let sid = subtree.uuid;
    db.add_group(root, subtree, t(1)).unwrap();
    db.move_group(sid, parent, t(2)).unwrap(); // 91 ancestors + 2 groups = 93.
    assert_eq!(db.find_group(leafid).unwrap().location_changed, t(0));
    let extra = Group::new("extra", t(0));
    let extraid = extra.uuid;
    db.add_group(parent, extra, t(1)).unwrap();
    let before = db.clone();
    assert!(db.move_group(sid, extraid, t(3)).is_err());
    assert_eq!(db, before);
    let mut g = Group::new("too tall", t(0));
    g.groups.push(Group::new("leaf", t(0)));
    assert!(db.add_group(extraid, g, t(1)).is_err());
    assert_eq!(db, before);
    // Default-stack mutable lookup, access, iteration, removal and tombstoning at depth 93.
    db.find_group_mut(leafid).unwrap().icon_id = 9;
    let e = make_entry("deep");
    let eid = e.uuid;
    db.add_entry(leafid, e, t(1)).unwrap();
    db.delete_entry(eid, t(2)).unwrap();
    assert!(db.is_in_recycle_bin(eid));
    db.restore_entry(eid, leafid, t(3)).unwrap();
    db.purge_entry(eid, t(4)).unwrap();
    db.purge_group(sid, t(5)).unwrap();
}

#[test]
fn insertion_stamps_location_changed() {
    let mut db = Database::new("test");
    let e = Entry::new(Timestamp(1));
    let eid = e.uuid;
    let g = Group::new("new", Timestamp(1));
    let gid = g.uuid;
    db.add_entry(db.root_id(), e, Timestamp(999)).unwrap();
    db.add_group(db.root_id(), g, Timestamp(999)).unwrap();
    assert_eq!(db.find_entry(eid).unwrap().location_changed, Timestamp(999));
    assert_eq!(db.find_group(gid).unwrap().location_changed, Timestamp(999));
}

#[test]
fn delete_deep_group_refuses_before_creating_recycle_bin() {
    let mut db = Database::new("root");
    let mut parent = db.root_id();
    let mut first = None;
    for _ in 0..92 {
        let g = Group::new("deep", t(0));
        let id = g.uuid;
        first.get_or_insert(id);
        db.add_group(parent, g, t(1)).unwrap();
        parent = id;
    }
    let before = db.clone();
    assert!(db.delete_group(first.unwrap(), t(2)).is_err());
    assert_eq!(db, before);
}

#[test]
fn public_trim_history_matches_mutation_policy() {
    for (items, size, want) in [
        (-1, -1, 3),
        (0, -1, 0),
        (2, -1, 2),
        (-1, 28, 2),
        (-1, 27, 1),
        (-1, 0, 0),
    ] {
        let mut entry = make_entry("current");
        for n in 0..3 {
            let mut h = make_entry("v0");
            h.modified = t(n);
            h.attachments.push(Attachment {
                name: "abc".into(),
                data: vec![0; 4],
                protected: true,
            });
            entry.history.push(h);
        }
        entry.trim_history(items, size);
        assert_eq!(entry.history.len(), want);
        assert_eq!(
            entry.history.iter().map(|h| h.modified).collect::<Vec<_>>(),
            ((3 - want)..3).map(|n| t(n as i64)).collect::<Vec<_>>()
        );
    }
    // Zero size retains zero-cost snapshots, exactly as update_entry does.
    let mut entry = make_entry("current");
    entry.history.push(Entry::new(t(0)));
    entry.trim_history(-1, 0);
    assert_eq!(entry.history.len(), 1);
}
// A vault whose recycle-bin UUID points at the root group (another client, or a corrupted meta)
// has no usable bin: treating the root as the bin would make every entry "recycled", turn deletes
// into permanent purges and make moves into the bin no-ops.
fn db_with_root_as_bin() -> Database {
    let mut db = Database::new("Root");
    db.meta.recycle_bin_enabled = true;
    db.meta.recycle_bin_uuid = Some(db.root_id());
    db
}
#[test]
fn root_as_recycle_bin_does_not_make_entries_recycled() {
    let mut db = db_with_root_as_bin();
    let e = make_entry("ordinary");
    let id = e.uuid;
    db.add_entry(db.root_id(), e, t(1)).unwrap();
    assert!(!db.is_in_recycle_bin(id));
}
#[test]
fn root_as_recycle_bin_delete_entry_moves_to_a_fresh_bin() {
    let mut db = db_with_root_as_bin();
    let e = make_entry("ordinary");
    let id = e.uuid;
    db.add_entry(db.root_id(), e, t(1)).unwrap();
    db.delete_entry(id, t(2)).unwrap();
    let bin = db.meta.recycle_bin_uuid.unwrap();
    assert_ne!(bin, db.root_id());
    assert_eq!(db.find_group(bin).unwrap().name, "Recycle Bin");
    assert_eq!(db.parent_of_entry(id), Some(bin));
    assert!(db.is_in_recycle_bin(id));
    assert!(db.deleted_objects.is_empty());
    // A second delete is now a real purge: the entry is inside the new bin.
    db.delete_entry(id, t(3)).unwrap();
    assert!(db.find_entry(id).is_none());
    assert_eq!(db.deleted_objects.len(), 1);
}
#[test]
fn root_as_recycle_bin_delete_group_moves_to_a_fresh_bin() {
    let mut db = db_with_root_as_bin();
    let mut g = Group::new("g", t(0));
    let e = make_entry("inside");
    let eid = e.uuid;
    g.entries.push(e);
    let gid = g.uuid;
    db.add_group(db.root_id(), g, t(1)).unwrap();
    db.delete_group(gid, t(2)).unwrap();
    let bin = db.meta.recycle_bin_uuid.unwrap();
    assert_ne!(bin, db.root_id());
    assert!(db
        .find_group(bin)
        .unwrap()
        .groups
        .iter()
        .any(|g| g.uuid == gid));
    assert!(db.is_in_recycle_bin(eid));
    assert!(db.deleted_objects.is_empty());
}

#[test]
fn add_entries_matches_add_entry_and_is_all_or_nothing() {
    let mut db = Database::new("t");
    let root = db.root_id();
    let batch = vec![make_entry("a"), make_entry("b")];
    let ids: Vec<Uuid> = batch.iter().map(|e| e.uuid).collect();
    db.add_entries(root, batch, t(5)).unwrap();
    for id in &ids {
        assert_eq!(db.find_entry(*id).unwrap().location_changed, t(5));
        assert_eq!(db.parent_of_entry(*id), Some(root));
    }
    // A repeated id inside the batch, an id already in the tree and a missing parent change nothing.
    let twin = make_entry("c");
    let dup = vec![twin.clone(), twin];
    assert!(db.add_entries(root, dup, t(6)).is_err());
    let existing = db.find_entry(ids[0]).unwrap().clone();
    assert!(db
        .add_entries(root, vec![make_entry("d"), existing], t(6))
        .is_err());
    assert!(db
        .add_entries(Uuid::nil(), vec![make_entry("e")], t(6))
        .is_err());
    assert_eq!(db.root.entries.len(), 2);
    db.add_entries(root, Vec::new(), t(7)).unwrap();
}
fn db_with_group_entry() -> (Database, Uuid, Uuid) {
    let mut db = Database::new("Root");
    let g = Group::new("work", t(0));
    let gid = g.uuid;
    db.add_group(db.root_id(), g, t(1)).unwrap();
    let e = make_entry("in work");
    let id = e.uuid;
    db.add_entry(gid, e, t(1)).unwrap();
    (db, id, gid)
}
#[test]
fn deleting_an_entry_records_its_group_and_restore_target_uses_it() {
    let (mut db, id, gid) = db_with_group_entry();
    assert_eq!(db.find_entry(id).unwrap().previous_parent, None);
    db.delete_entry(id, t(2)).unwrap();
    assert!(db.is_in_recycle_bin(id));
    assert_eq!(db.find_entry(id).unwrap().previous_parent, Some(gid));
    assert_eq!(db.restore_target_of_entry(id), gid);
    db.restore_entry(id, gid, t(3)).unwrap();
    assert_eq!(db.parent_of_entry(id), Some(gid));
    // Like every parent change, the restore records its immediate source: the bin.
    assert_eq!(
        db.find_entry(id).unwrap().previous_parent,
        db.meta.recycle_bin_uuid
    );
}
#[test]
fn moving_inside_the_bin_keeps_the_original_origin() {
    let (mut db, id, gid) = db_with_group_entry();
    db.delete_entry(id, t(2)).unwrap();
    let bin = db.meta.recycle_bin_uuid.unwrap();
    let sub = Group::new("sub", t(0));
    let sub_id = sub.uuid;
    db.add_group(bin, sub, t(3)).unwrap();
    db.move_entry(id, sub_id, t(4)).unwrap();
    assert_eq!(db.find_entry(id).unwrap().previous_parent, Some(gid));
    assert_eq!(db.restore_target_of_entry(id), gid);
}
#[test]
fn an_ordinary_move_records_the_immediate_source() {
    let (mut db, id, gid) = db_with_group_entry();
    let root = db.root_id();
    db.move_entry(id, root, t(2)).unwrap();
    assert_eq!(db.find_entry(id).unwrap().previous_parent, Some(gid));
}
#[test]
fn a_move_replaces_a_previous_parent_that_another_client_recorded() {
    // KeePass moved the entry from A to B and recorded A; MyPass then moves it from B to C.
    let mut db = Database::new("Root");
    let (a, b, c) = (
        Group::new("a", t(0)),
        Group::new("b", t(0)),
        Group::new("c", t(0)),
    );
    let (a_id, b_id, c_id) = (a.uuid, b.uuid, c.uuid);
    for g in [a, b, c] {
        db.add_group(db.root_id(), g, t(1)).unwrap();
    }
    let e = make_entry("moved elsewhere");
    let id = e.uuid;
    db.add_entry(b_id, e, t(1)).unwrap();
    db.find_entry_mut(id).unwrap().previous_parent = Some(a_id);
    db.move_entry(id, c_id, t(2)).unwrap();
    assert_eq!(db.find_entry(id).unwrap().previous_parent, Some(b_id));
    // Same-parent operations keep what was recorded.
    db.move_entry(id, c_id, t(3)).unwrap();
    assert_eq!(db.find_entry(id).unwrap().previous_parent, Some(b_id));
}
#[test]
fn restore_followed_by_another_move_and_a_second_delete_records_each_source() {
    let (mut db, id, gid) = db_with_group_entry();
    let root = db.root_id();
    let bin_of = |db: &Database| db.meta.recycle_bin_uuid;
    db.delete_entry(id, t(2)).unwrap();
    db.restore_entry(id, gid, t(3)).unwrap();
    assert_eq!(db.find_entry(id).unwrap().previous_parent, bin_of(&db));
    db.move_entry(id, root, t(4)).unwrap();
    assert_eq!(db.find_entry(id).unwrap().previous_parent, Some(gid));
    db.delete_entry(id, t(5)).unwrap();
    assert_eq!(db.find_entry(id).unwrap().previous_parent, Some(root));
    assert_eq!(db.restore_target_of_entry(id), root);
}
#[test]
fn a_group_move_records_its_immediate_source_and_bin_moves_keep_the_origin() {
    let mut db = Database::new("Root");
    let (a, b) = (Group::new("a", t(0)), Group::new("b", t(0)));
    let (a_id, b_id) = (a.uuid, b.uuid);
    db.add_group(db.root_id(), a, t(1)).unwrap();
    db.add_group(db.root_id(), b, t(1)).unwrap();
    let child = Group::new("child", t(0));
    let child_id = child.uuid;
    db.add_group(a_id, child, t(1)).unwrap();
    db.find_group_mut(child_id).unwrap().previous_parent = Some(Uuid::new_v4());
    db.move_group(child_id, b_id, t(2)).unwrap();
    assert_eq!(db.find_group(child_id).unwrap().previous_parent, Some(a_id));
    db.move_group(child_id, b_id, t(3)).unwrap();
    assert_eq!(db.find_group(child_id).unwrap().previous_parent, Some(a_id));
    db.delete_group(child_id, t(4)).unwrap();
    assert_eq!(db.find_group(child_id).unwrap().previous_parent, Some(b_id));
    let bin = db.meta.recycle_bin_uuid.unwrap();
    let sub = Group::new("sub", t(0));
    let sub_id = sub.uuid;
    db.add_group(bin, sub, t(5)).unwrap();
    db.move_group(child_id, sub_id, t(6)).unwrap();
    assert_eq!(db.find_group(child_id).unwrap().previous_parent, Some(b_id));
}
#[test]
fn restore_target_falls_back_to_the_root_when_the_original_group_is_gone() {
    let (mut db, id, gid) = db_with_group_entry();
    db.delete_entry(id, t(2)).unwrap();
    // Purged outright: the recorded group no longer exists.
    db.purge_group(gid, t(3)).unwrap();
    assert_eq!(db.find_entry(id).unwrap().previous_parent, Some(gid));
    assert_eq!(db.restore_target_of_entry(id), db.root_id());
    // Never recorded at all.
    assert_eq!(db.restore_target_of_entry(Uuid::nil()), db.root_id());
}
#[test]
fn restore_target_ignores_an_original_group_that_was_recycled_too() {
    let (mut db, id, gid) = db_with_group_entry();
    db.delete_entry(id, t(2)).unwrap();
    db.delete_group(gid, t(3)).unwrap();
    // The group exists, but inside the bin: restoring there would leave the entry deleted.
    assert!(db.find_group(gid).is_some());
    assert_eq!(db.restore_target_of_entry(id), db.root_id());
}
#[test]
fn restore_target_never_names_the_bin() {
    let (mut db, id, _) = db_with_group_entry();
    db.delete_entry(id, t(2)).unwrap();
    let bin = db.meta.recycle_bin_uuid.unwrap();
    db.find_entry_mut(id).unwrap().previous_parent = Some(bin);
    assert_eq!(db.restore_target_of_entry(id), db.root_id());
}
#[test]
fn deleting_a_group_records_its_parent_and_restore_group_uses_it() {
    let mut db = Database::new("Root");
    let parent = Group::new("parent", t(0));
    let parent_id = parent.uuid;
    db.add_group(db.root_id(), parent, t(1)).unwrap();
    let child = Group::new("child", t(0));
    let child_id = child.uuid;
    db.add_group(parent_id, child, t(1)).unwrap();
    db.delete_group(child_id, t(2)).unwrap();
    assert!(db.is_group_in_recycle_bin(child_id));
    assert_eq!(
        db.find_group(child_id).unwrap().previous_parent,
        Some(parent_id)
    );
    let target = db.restore_target_of_group(child_id);
    assert_eq!(target, parent_id);
    db.restore_group(child_id, target, t(3)).unwrap();
    assert!(!db.is_group_in_recycle_bin(child_id));
    assert_eq!(
        db.find_group(child_id).unwrap().previous_parent,
        db.meta.recycle_bin_uuid
    );
    assert!(db
        .find_group(parent_id)
        .unwrap()
        .groups
        .iter()
        .any(|g| g.uuid == child_id));
}
#[test]
fn restore_group_with_a_missing_original_parent_goes_to_the_root() {
    let mut db = Database::new("Root");
    let parent = Group::new("parent", t(0));
    let parent_id = parent.uuid;
    db.add_group(db.root_id(), parent, t(1)).unwrap();
    let mut child = Group::new("child", t(0));
    let e = make_entry("nested");
    let eid = e.uuid;
    child.entries.push(e);
    let child_id = child.uuid;
    db.add_group(parent_id, child, t(1)).unwrap();
    db.delete_group(child_id, t(2)).unwrap();
    db.purge_group(parent_id, t(3)).unwrap();
    let target = db.restore_target_of_group(child_id);
    assert_eq!(target, db.root_id());
    db.restore_group(child_id, target, t(4)).unwrap();
    assert_eq!(db.parent_of_entry(eid), Some(child_id));
    assert!(!db.is_in_recycle_bin(eid));
}
#[test]
fn restore_group_rejects_groups_outside_the_bin_and_the_bin_itself() {
    let (mut db, _, gid) = db_with_group_entry();
    let root = db.root_id();
    assert!(db.restore_group(gid, root, t(2)).is_err());
    let doomed = Group::new("doomed", t(0));
    let did = doomed.uuid;
    db.add_group(root, doomed, t(1)).unwrap();
    db.delete_group(did, t(2)).unwrap();
    let bin = db.meta.recycle_bin_uuid.unwrap();
    assert!(db.restore_group(bin, root, t(3)).is_err());
    assert!(db.restore_group(root, root, t(3)).is_err());
}
