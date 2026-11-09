//! Pure two-way merge, following SyncKit's numbered rules (brief rules 1–7).
use mypass_kdbx::{Database, DeletedObject, Entry, Group, Timestamp, MAX_GROUP_DEPTH};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use uuid::Uuid;

#[derive(Default)]
struct Index<'a> {
    groups: HashMap<Uuid, (&'a Group, Uuid)>,
    entries: HashMap<Uuid, (&'a Entry, Uuid)>,
}
impl<'a> Index<'a> {
    fn new(root: &'a Group, canonical_root: Uuid) -> Self {
        let mut result = Self::default();
        let mut stack = vec![(root, canonical_root)];
        let mut seen = HashSet::new();
        // Iterative preorder: duplicate UUIDs keep the first occurrence in tree order.
        // Still visit duplicate groups' contents so unrelated live children are retained.
        while let Some((g, parent)) = stack.pop() {
            let id = if std::ptr::eq(g, root) {
                canonical_root
            } else {
                g.uuid
            };
            if seen.insert(id) {
                result.groups.insert(id, (g, parent));
            }
            for e in &g.entries {
                if seen.insert(e.uuid) {
                    result.entries.insert(e.uuid, (e, id));
                }
            }
            stack.extend(g.groups.iter().rev().map(|c| (c, id)));
        }
        result
    }
}

fn metadata(g: &Group) -> Group {
    // Do not clone the recursive tree: caller-created trees need not obey XML limits.
    Group {
        uuid: g.uuid,
        name: g.name.clone(),
        icon_id: g.icon_id,
        created: g.created,
        modified: g.modified,
        accessed: g.accessed,
        location_changed: g.location_changed,
        expires: g.expires,
        expiry_time: g.expiry_time,
        custom_data: g.custom_data.clone(),
        extra: g.extra.clone(),
        entries: vec![],
        groups: vec![],
    }
}
fn version(e: &Entry) -> impl std::hash::Hash + Eq + '_ {
    (
        e.uuid,
        e.modified,
        e.fields
            .iter()
            .map(|f| (f.key.as_str(), f.value.as_str(), f.protected))
            .collect::<Vec<_>>(),
        e.attachments
            .iter()
            .map(|a| (a.name.as_str(), a.data.as_slice(), a.protected))
            .collect::<Vec<_>>(),
    )
}
fn merge_entry(l: &Entry, r: &Entry) -> Entry {
    // Rule 2: whole-second mtimes can tie; keep local current without losing a
    // distinct remote version. Attachment-only edits must survive deduplication.
    let (winner, loser) = if r.modified > l.modified {
        (r, l)
    } else {
        (l, r)
    };
    let mut out = winner.clone();
    let mut seen = HashSet::new();
    // Swift parity: retain existing snapshots even when identical to current.
    out.history = winner
        .history
        .iter()
        .chain(&loser.history)
        .chain((version(winner) != version(loser)).then_some(loser))
        .filter(|e| seen.insert(version(e)))
        .map(|e| {
            let mut s = e.clone();
            if std::ptr::eq(e, loser) {
                s.history.clear();
            }
            s
        })
        .collect();
    // Stable oldest-first: equal timestamps keep winner/loser union order.
    out.history.sort_by_key(|e| e.modified);
    out
}

// Walk each functional-graph component once. A path-local position map
// distinguishes actual cycle members from tails feeding into the cycle.
fn cycle_roots(
    parents: &BTreeMap<Uuid, Uuid>,
    visited: &HashSet<Uuid>,
    groups: &HashMap<Uuid, Group>,
) -> Vec<Uuid> {
    let mut done = visited.clone();
    let mut roots = vec![];
    for &start in parents.keys() {
        let mut positions = HashMap::new();
        let mut path = vec![];
        let mut cursor = start;
        while !done.contains(&cursor) {
            if let Some(&offset) = positions.get(&cursor) {
                if let Some(&id) = path.get(offset..).and_then(|cycle| {
                    cycle
                        .iter()
                        .min_by_key(|id| (groups.get(id).map(|g| g.location_changed), **id))
                }) {
                    roots.push(id);
                }
                break;
            }
            positions.insert(cursor, path.len());
            path.push(cursor);
            let Some(&parent) = parents.get(&cursor) else {
                break;
            };
            cursor = parent;
        }
        done.extend(path);
    }
    roots.sort();
    roots
}

/// Merge without I/O or mutation. Children and tombstones are sorted by UUID bytes.
/// Duplicate UUIDs within either side keep the first preorder occurrence. Group
/// parents follow the greater location_changed clock, with local winning ties.
/// Only the oldest move in each parent cycle attaches to root (UUID tie-break); descendants
/// retain their parents. Missing parents attach to root. Output depth is capped at
/// MAX_GROUP_DEPTH by reattaching deeper groups to root, retaining their contents.
pub fn merge(local: &Database, remote: &Database) -> Database {
    let root = local.root.uuid;
    // D1 / Swift rule 9: unlike Swift's early return, alias different roots and
    // merge all their children; the local root's identity and metadata remain.
    let l = Index::new(&local.root, root);
    let mut r = Index::new(&remote.root, root);
    // For malformed cross-type collisions across sides, the local object type
    // wins; unrelated children of a suppressed group remain live via D2.
    r.groups.retain(|id, _| !l.entries.contains_key(id));
    r.entries.retain(|id, _| !l.groups.contains_key(id));
    // Rule 7 (brief 5): union tombstones, retaining the later deletion time.
    let mut deleted = BTreeMap::<Uuid, Timestamp>::new();
    for d in local.deleted_objects.iter().chain(&remote.deleted_objects) {
        deleted
            .entry(d.uuid)
            .and_modify(|t| *t = (*t).max(d.deleted_at))
            .or_insert(d.deleted_at);
    }
    // Rule 5: use the surviving (newest) modification, not just the first side.
    deleted.retain(|id, time| {
        l.entries
            .get(id)
            .into_iter()
            .chain(r.entries.get(id))
            .map(|(e, _)| e.modified)
            .chain(
                l.groups
                    .get(id)
                    .into_iter()
                    .chain(r.groups.get(id))
                    .map(|(g, _)| g.modified),
            )
            .max()
            .is_none_or(|modified| modified <= *time)
    });
    // The root is always live, including when a peer tombstones its UUID.
    deleted.remove(&root);
    let ids: HashSet<_> = l.groups.keys().chain(r.groups.keys()).copied().collect();
    let mut groups = HashMap::new();
    let mut parents = BTreeMap::new();
    for id in ids {
        if id != root && deleted.contains_key(&id) {
            continue;
        }
        let a = l.groups.get(&id);
        let b = r.groups.get(&id);
        // Rule 6 applies to groups as well as entries. Content and location
        // clocks arbitrate independently; preserve the winning parent's clock.
        let (parent, location_changed) = match (a, b) {
            (Some((a, ap)), Some((b, bp))) => {
                if a.location_changed >= b.location_changed {
                    (*ap, a.location_changed)
                } else {
                    (*bp, b.location_changed)
                }
            }
            (Some((g, p)), None) | (None, Some((g, p))) => (*p, g.location_changed),
            (None, None) => continue,
        };
        let (newer, older) = match (a, b) {
            (Some((a, _)), Some((b, _)))
                if a.modified >= b.modified
                    || (id == root && local.root.uuid != remote.root.uuid) =>
            {
                (*a, Some(*b))
            }
            (Some((a, _)), Some((b, _))) => (*b, Some(*a)),
            (Some((a, _)), None) | (None, Some((a, _))) => (*a, None),
            (None, None) => continue,
        };
        // Rule 3: metadata and custom-data union are independent of child merge.
        let mut g = metadata(newer);
        g.uuid = id;
        g.location_changed = location_changed;
        let mut custom = BTreeMap::new();
        for c in older
            .into_iter()
            .flat_map(|g| &g.custom_data)
            .chain(&newer.custom_data)
        {
            custom.insert(c.key.clone(), c.clone());
        }
        g.custom_data = custom.into_values().collect();
        groups.insert(id, g);
        if id != root {
            parents.insert(id, parent);
        }
    }
    // Rule 4: assemble the UUID union once, using adjacency indexes rather than
    // rescanning every entry for every group. D2: missing parents attach to root.
    let mut children: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for (&id, &parent) in &parents {
        let parent = if groups.contains_key(&parent) {
            parent
        } else {
            root
        };
        children.entry(parent).or_default().push(id);
    }
    let mut queue = VecDeque::from([(root, 1usize)]);
    let mut order = vec![];
    let mut actual_parents = HashMap::new();
    let mut visited = HashSet::new();
    loop {
        while let Some((id, depth)) = queue.pop_front() {
            if !visited.insert(id) {
                continue;
            }
            order.push(id);
            for child in children.remove(&id).unwrap_or_default() {
                // The old edge into a cycle's detached member must stay cut.
                if visited.contains(&child) {
                    continue;
                }
                let (parent, next_depth) = if depth >= MAX_GROUP_DEPTH {
                    (root, 2)
                } else {
                    (id, depth + 1)
                };
                actual_parents.insert(child, parent);
                queue.push_back((child, next_depth));
            }
        }
        if visited.len() == groups.len() {
            break;
        }
        // D2: missing/deleted parents were already rooted above. Every remaining
        // component feeds a cycle. Cut only its oldest move, then resume the
        // same depth-limited BFS at depth 2, preserving all other relationships.
        for id in cycle_roots(&parents, &visited, &groups) {
            if !visited.contains(&id) {
                actual_parents.insert(id, root);
                queue.push_back((id, 2));
            }
        }
        if queue.is_empty() {
            break;
        }
    }
    let entries: HashSet<_> = l.entries.keys().chain(r.entries.keys()).copied().collect();
    for id in entries {
        if deleted.contains_key(&id) || groups.contains_key(&id) {
            continue;
        }
        let (entry, parent) = match (l.entries.get(&id), r.entries.get(&id)) {
            (Some((a, ap)), Some((b, bp))) => {
                // Rule 6: location, independently of content; ties favour local.
                let (parent, location) = if a.location_changed >= b.location_changed {
                    (*ap, a.location_changed)
                } else {
                    (*bp, b.location_changed)
                };
                let mut entry = merge_entry(a, b);
                entry.location_changed = location;
                (entry, parent)
            }
            (Some((a, p)), None) | (None, Some((a, p))) => ((*a).clone(), *p), // Rule 1
            (None, None) => continue,
        };
        // D2: adding an entry does not bump its parent's mtime. A tombstoned
        // parent must not silently swallow that still-live entry.
        let parent = if groups.contains_key(&parent) {
            parent
        } else {
            root
        };
        if let Some(g) = groups.get_mut(&parent) {
            g.entries.push(entry);
        }
    }
    for g in groups.values_mut() {
        g.entries.sort_by_key(|e| e.uuid);
    }
    for id in order.into_iter().rev() {
        if id == root {
            continue;
        }
        if let Some(mut g) = groups.remove(&id) {
            g.groups.sort_by_key(|g| g.uuid);
            if let Some(parent) = actual_parents.get(&id).and_then(|p| groups.get_mut(p)) {
                parent.groups.push(g);
            }
        }
    }
    let mut merged_root = groups
        .remove(&root)
        .unwrap_or_else(|| metadata(&local.root));
    merged_root.groups.sort_by_key(|g| g.uuid);
    // Rule 8 (brief 7): nil means unset. All other meta/settings stay local.
    let mut meta = local.meta.clone();
    meta.recycle_bin_uuid = local
        .meta
        .recycle_bin_uuid
        .filter(|u| !u.is_nil())
        .or(remote.meta.recycle_bin_uuid.filter(|u| !u.is_nil()));
    Database {
        root: merged_root,
        meta,
        deleted_objects: deleted
            .into_iter()
            .map(|(uuid, deleted_at)| DeletedObject { uuid, deleted_at })
            .collect(),
        settings: local.settings.clone(),
        dropped_attachment_refs: local.dropped_attachment_refs,
        dropped_protected_unknown: local.dropped_protected_unknown,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use mypass_kdbx::{Attachment, KdfParams};
    fn make_entry(id: u128, title: &str, time: i64) -> Entry {
        let mut e = Entry::new(Timestamp(time));
        e.uuid = Uuid::from_u128(id);
        e.set_field("Title", title, false);
        e
    }
    fn group(id: u128) -> Group {
        let mut g = Group::new("MyPass", Timestamp(1000));
        g.uuid = Uuid::from_u128(id);
        g
    }
    fn make_db(entries: Vec<Entry>) -> Database {
        let mut d = Database::new("MyPass");
        d.root = group(100);
        d.root.entries = entries;
        d.settings.kdf = KdfParams::AesKdf {
            seed: vec![0; 32],
            rounds: 1,
        };
        d
    }
    fn att(name: &str, byte: u8) -> Attachment {
        Attachment {
            name: name.into(),
            data: vec![byte],
            protected: true,
        }
    }
    fn pair(lt: i64, rt: i64) -> (Database, Database) {
        (
            make_db(vec![make_entry(1, "Local", lt)]),
            make_db(vec![make_entry(1, "Remote", rt)]),
        )
    }
    #[test]
    fn added_only_locally() {
        let (l, _) = pair(2000, 1000);
        assert_eq!(merge(&l, &make_db(vec![])), l);
    }
    #[test]
    fn added_only_remotely() {
        let (l, _) = pair(2000, 1000);
        assert_eq!(merge(&make_db(vec![]), &l), l);
    }
    fn edits(lt: i64, rt: i64, title: &str, loser: &str) {
        let (l, r) = pair(lt, rt);
        let m = merge(&l, &r);
        let e = &m.root.entries[0];
        assert_eq!(e.title(), title);
        assert_eq!(e.history.len(), 1);
        assert_eq!(e.history[0].title(), loser);
        assert!(e.history[0].history.is_empty());
    }
    #[test]
    fn edited_both_newer_local_wins() {
        edits(3000, 1000, "Local", "Remote");
    }
    #[test]
    fn edited_both_newer_remote_wins() {
        edits(1000, 3000, "Remote", "Local");
    }
    #[test]
    fn edited_equal_timestamps_prefers_local() {
        edits(5000, 5000, "Local", "Remote");
    }
    #[test]
    fn group_metadata_merge_independent_of_child_merge() {
        let mut l = make_db(vec![]);
        let mut r = l.clone();
        let mut a = group(2);
        a.name = "RenamedByLocal".into();
        a.modified = Timestamp(4000);
        a.icon_id = 1;
        let mut b = group(2);
        b.name = "OriginalName".into();
        b.modified = Timestamp(1500);
        b.entries.push(make_entry(1, "AddedByRemote", 2000));
        l.root.groups.push(a.clone());
        r.root.groups.push(b);
        let m = merge(&l, &r);
        assert_eq!(m.root.groups[0].name, a.name);
        assert_eq!(m.root.groups[0].icon_id, 1);
        assert_eq!(m.root.groups[0].entries.len(), 1);
        assert_eq!(merge(&r, &l).root.groups[0], m.root.groups[0]);
    }
    #[test]
    fn group_custom_data_unions_distinct_keys_and_prefers_newer_on_conflict() {
        let mut l = make_db(vec![]);
        let mut r = l.clone();
        l.root.modified = Timestamp(2000);
        r.root.modified = Timestamp(3000);
        l.root.set_custom("local", "a");
        l.root.set_custom("same", "old");
        r.root.set_custom("remote", "b");
        r.root.set_custom("same", "new");
        let m = merge(&l, &r);
        assert_eq!(m.root.custom("local"), Some("a"));
        assert_eq!(m.root.custom("remote"), Some("b"));
        assert_eq!(m.root.custom("same"), Some("new"));
        r.root.modified = l.root.modified;
        assert_eq!(merge(&l, &r).root.custom("same"), Some("old"));
    }
    fn deletion(local_delete: bool, modified: i64, deleted: i64, live: bool) {
        let a = make_db(vec![make_entry(1, "edit", modified)]);
        let mut b = make_db(vec![]);
        b.deleted_objects.push(DeletedObject {
            uuid: Uuid::from_u128(1),
            deleted_at: Timestamp(deleted),
        });
        let m = if local_delete {
            merge(&b, &a)
        } else {
            merge(&a, &b)
        };
        assert_eq!(m.root.entries.len(), usize::from(live));
        assert_eq!(m.deleted_objects.is_empty(), live);
    }
    #[test]
    fn delete_local_edit_remote_newer_resurrects() {
        deletion(true, 3000, 2000, true);
    }
    #[test]
    fn delete_local_edit_remote_older_deletion_wins() {
        deletion(true, 1000, 5000, false);
    }
    #[test]
    fn delete_remote_edit_local_newer_resurrects() {
        deletion(false, 3000, 2000, true);
    }
    #[test]
    fn delete_remote_edit_local_older_deletion_wins() {
        deletion(false, 1000, 5000, false);
    }
    #[test]
    fn delete_equal_time_deletion_wins() {
        deletion(true, 2000, 2000, false);
    }
    fn moved(lt: i64, rt: i64, want: usize) {
        let mut l = make_db(vec![]);
        let mut r = l.clone();
        l.root.groups = vec![group(2), group(3)];
        r.root.groups = l.root.groups.clone();
        let mut e = make_entry(1, "Moved", 1000);
        e.location_changed = Timestamp(lt);
        l.root.groups[1].entries.push(e.clone());
        e.location_changed = Timestamp(rt);
        r.root.groups[0].entries.push(e);
        let m = merge(&l, &r);
        assert_eq!(m.root.groups[want].entries.len(), 1);
        assert_eq!(m.root.groups[1 - want].entries.len(), 0);
    }
    #[test]
    fn move_local_newer_location_wins() {
        moved(9000, 1000, 1);
    }
    #[test]
    fn move_remote_newer_location_wins() {
        moved(1000, 9000, 0);
    }
    #[test]
    fn move_tie_prefers_local() {
        moved(1000, 1000, 1);
    }
    fn group_moved(lt: i64, rt: i64, want_remote: bool) {
        let mut l = make_db(vec![]);
        let mut r = l.clone();
        let mut a = group(5);
        a.location_changed = Timestamp(lt);
        let mut bin = group(9);
        bin.groups.push(a.clone());
        l.root.groups.push(bin);
        a.location_changed = Timestamp(rt);
        r.root.groups = vec![a, group(9)];
        let m = merge(&l, &r);
        if want_remote {
            assert_eq!(m.root.groups[0].uuid, Uuid::from_u128(5));
            assert_eq!(m.root.groups[0].location_changed, Timestamp(rt));
        } else {
            assert_eq!(m.root.groups[0].groups[0].uuid, Uuid::from_u128(5));
            assert_eq!(m.root.groups[0].groups[0].location_changed, Timestamp(lt));
        }
    }
    #[test]
    fn group_move_remote_newer_location_wins() {
        group_moved(1000, 9000, true);
    }
    #[test]
    fn group_move_local_newer_location_wins() {
        group_moved(9000, 1000, false);
    }
    #[test]
    fn group_move_tie_prefers_local() {
        group_moved(1000, 1000, false);
    }
    #[test]
    fn recycle_bin_restore_of_group_is_not_reverted() {
        let mut l = make_db(vec![]);
        let mut r = l.clone();
        let mut bin = group(9);
        bin.groups.push(group(5));
        l.root.groups.push(bin);
        l.meta.recycle_bin_uuid = Some(Uuid::from_u128(9));
        r.root.groups = vec![group(5), group(9)];
        r.root.groups[0].location_changed = Timestamp(9000);
        let m = merge(&l, &r);
        assert_eq!(m.root.groups[0].uuid, Uuid::from_u128(5));
        assert_eq!(m.root, merge(&r, &l).root);
    }
    #[test]
    fn location_changed_follows_the_winning_parent() {
        let mut l = make_db(vec![make_entry(1, "new content", 8000)]);
        let mut r = make_db(vec![]);
        l.root.entries[0].location_changed = Timestamp(1000);
        let mut target = group(2);
        let mut e = make_entry(1, "old content", 2000);
        e.location_changed = Timestamp(9000);
        target.entries.push(e);
        r.root.groups.push(target);
        let mut g = group(5);
        g.modified = Timestamp(8000);
        g.location_changed = Timestamp(1000);
        l.root.groups.push(g.clone());
        g.modified = Timestamp(2000);
        g.location_changed = Timestamp(9000);
        r.root.groups[0].groups.push(g);
        let m = merge(&l, &r);
        let e = &m.root.groups[0].entries[0];
        assert_eq!(e.title(), "new content");
        assert_eq!(e.location_changed, Timestamp(9000));
        let g = &m.root.groups[0].groups[0];
        assert_eq!(g.modified, Timestamp(8000));
        assert_eq!(g.location_changed, Timestamp(9000));
    }
    #[test]
    fn root_tombstone_is_dropped() {
        let mut l = make_db(vec![]);
        l.deleted_objects.push(DeletedObject {
            uuid: l.root.uuid,
            deleted_at: Timestamp(9000),
        });
        let m = merge(&l, &l);
        assert_eq!(m.root.uuid, l.root.uuid);
        assert!(m.deleted_objects.is_empty());
    }
    #[test]
    fn deleted_objects_union_keeps_later_deletion_time() {
        let mut l = make_db(vec![]);
        let mut r = l.clone();
        l.deleted_objects.push(DeletedObject {
            uuid: Uuid::from_u128(1),
            deleted_at: Timestamp(1000),
        });
        r.deleted_objects.push(DeletedObject {
            uuid: Uuid::from_u128(1),
            deleted_at: Timestamp(9000),
        });
        assert_eq!(merge(&l, &r).deleted_objects, r.deleted_objects);
        assert_eq!(merge(&r, &l).deleted_objects, r.deleted_objects);
    }
    #[test]
    fn recycle_bin_uuid_prefers_non_nil_local_first() {
        for a in [None, Some(Uuid::nil()), Some(Uuid::from_u128(2))] {
            for b in [None, Some(Uuid::nil()), Some(Uuid::from_u128(3))] {
                let mut l = make_db(vec![]);
                let mut r = l.clone();
                l.meta.recycle_bin_uuid = a;
                r.meta.recycle_bin_uuid = b;
                assert_eq!(
                    merge(&l, &r).meta.recycle_bin_uuid,
                    a.filter(|u| !u.is_nil()).or(b.filter(|u| !u.is_nil()))
                );
            }
        }
    }
    fn attachments(lt: i64, rt: i64) -> (Database, Database) {
        let (mut l, mut r) = pair(lt, rt);
        l.root.entries[0].attachments.push(att("old.txt", 1));
        r.root.entries[0].attachments.push(att("new.txt", 2));
        (l, r)
    }
    #[test]
    fn newer_side_attachments_win_older_goes_to_history_with_its_attachments() {
        let (l, r) = attachments(2000, 3000);
        let m = merge(&l, &r);
        assert_eq!(m.root.entries[0].attachments, r.root.entries[0].attachments);
        assert_eq!(
            m.root.entries[0].history[0].attachments,
            l.root.entries[0].attachments
        );
    }
    #[test]
    fn entry_only_on_one_side_keeps_attachments() {
        let (l, _) = attachments(2000, 3000);
        assert_eq!(merge(&l, &make_db(vec![])), l);
        assert_eq!(merge(&make_db(vec![]), &l), l);
    }
    #[test]
    fn equal_timestamps_keep_local_attachments() {
        let (l, r) = attachments(2000, 2000);
        assert_eq!(
            merge(&l, &r).root.entries[0].attachments,
            l.root.entries[0].attachments
        );
    }
    #[test]
    fn attachment_only_difference_is_not_deduplicated_away_from_history() {
        let mut l = make_db(vec![make_entry(1, "T", 2000)]);
        l.root.entries[0].attachments.push(att("x", 1));
        let mut r = make_db(vec![make_entry(1, "T", 3000)]);
        r.root.entries[0].history.push(make_entry(1, "T", 2000));
        assert_eq!(merge(&l, &r).root.entries[0].history.len(), 2);
    }
    #[test]
    fn losing_entrys_own_history_is_unioned_into_winners() {
        let (mut l, mut r) = pair(2000, 3000);
        l.root.entries[0].history.push(make_entry(1, "l1", 1600));
        r.root.entries[0].history.push(make_entry(1, "w1", 1500));
        let m = merge(&l, &r);
        assert_eq!(
            m.root.entries[0]
                .history
                .iter()
                .map(|e| e.modified.0)
                .collect::<Vec<_>>(),
            vec![1500, 1600, 2000]
        );
    }
    #[test]
    fn snapshots_present_on_both_sides_are_not_duplicated() {
        let (mut l, mut r) = pair(2000, 3000);
        let s = make_entry(1, "s", 1500);
        l.root.entries[0].history.push(s.clone());
        r.root.entries[0].history.push(s);
        assert_eq!(merge(&l, &r).root.entries[0].history.len(), 2);
    }
    #[test]
    fn equal_timestamps_keep_distinct_remote_version_in_history() {
        let (l, r) = attachments(2000, 2000);
        let m = merge(&l, &r);
        assert_eq!(m.root.entries[0].history.len(), 1);
        assert_eq!(
            m.root.entries[0].history[0].attachments,
            r.root.entries[0].attachments
        );
    }
    #[test]
    fn equal_timestamps_identical_content_add_no_history() {
        let (l, _) = attachments(2000, 2000);
        assert!(merge(&l, &l).root.entries[0].history.is_empty());
    }
    #[test]
    fn merged_database_survives_save_and_open() {
        let (l, r) = attachments(2000, 3000);
        let m = merge(&l, &r);
        let p = secrecy::SecretString::from("pw");
        assert_eq!(Database::open(&m.save(&p).unwrap(), &p).unwrap(), m);
    }
    #[test]
    fn different_root_uuids_still_merge_children() {
        let (l, mut r) = pair(2000, 3000);
        r.root.uuid = Uuid::from_u128(101);
        r.root.modified = Timestamp(9000);
        r.root.name = "remote".into();
        r.root.entries[0].uuid = Uuid::from_u128(2);
        let m = merge(&l, &r);
        assert_eq!(m.root.entries.len(), 2);
        assert_eq!(m.root.name, l.root.name);
        assert_eq!(m.root.uuid, l.root.uuid);
    }
    #[test]
    fn remote_entry_added_to_locally_deleted_group_is_reattached_to_root() {
        let mut l = make_db(vec![]);
        let mut r = l.clone();
        let mut g = group(2);
        g.entries.push(make_entry(1, "Added", 2000));
        r.root.groups.push(g);
        l.deleted_objects.push(DeletedObject {
            uuid: Uuid::from_u128(2),
            deleted_at: Timestamp(3000),
        });
        let m = merge(&l, &r);
        assert_eq!(m.root.entries.len(), 1);
        assert!(m.root.groups.is_empty());
    }
    #[test]
    fn tombstoned_entry_inside_surviving_group_is_removed() {
        let (l, mut r) = pair(1000, 1000);
        r.deleted_objects.push(DeletedObject {
            uuid: Uuid::from_u128(1),
            deleted_at: Timestamp(3000),
        });
        assert!(merge(&l, &r).root.entries.is_empty());
    }
    #[test]
    fn merge_does_not_mutate_arguments() {
        let (l, r) = pair(2000, 3000);
        let before = (l.clone(), r.clone());
        let _ = merge(&l, &r);
        assert_eq!((l, r), before);
    }
    #[test]
    fn merge_is_idempotent() {
        let (l, r) = pair(2000, 3000);
        let m = merge(&l, &r);
        assert_eq!(merge(&l, &m), m);
    }
    #[test]
    fn merge_with_self_is_identity_up_to_ordering() {
        let l = make_db(vec![make_entry(1, "A", 1000), make_entry(2, "B", 2000)]);
        assert_eq!(merge(&l, &l), l);
    }
    #[test]
    fn duplicate_uuids_keep_first_tree_occurrence() {
        let mut l = make_db(vec![
            make_entry(1, "first", 1000),
            make_entry(1, "second", 9000),
        ]);
        let mut a = group(2);
        a.name = "first".into();
        let mut b = group(2);
        b.name = "second".into();
        b.entries.push(make_entry(3, "retained", 1000));
        l.root.groups = vec![a, b];
        let m = merge(&l, &make_db(vec![]));
        assert_eq!(m.root.entries.len(), 1);
        assert_eq!(m.root.entries[0].title(), "first");
        assert_eq!(m.root.groups.len(), 1);
        assert_eq!(m.root.groups[0].name, "first");
        assert_eq!(m.root.groups[0].entries.len(), 1);
    }
    #[test]
    fn cyclic_parent_union_is_reattached_without_duplicates() {
        let mut l = make_db(vec![]);
        let mut a = group(2);
        a.groups.push(group(3));
        l.root.groups.push(a);
        let mut r = make_db(vec![]);
        let mut b = group(3);
        b.groups.push(group(2));
        r.root.groups.push(b);
        let m = merge(&l, &r);
        assert_eq!(m.root.groups.len(), 1);
        assert_eq!(m.root.groups[0].uuid, Uuid::from_u128(2));
        assert_eq!(m.root.groups[0].groups[0].uuid, Uuid::from_u128(3));
        // A repeated ancestor UUID in a value tree cannot create a traversal loop.
        l.root.groups[0].groups[0].groups.push(group(2));
        assert_eq!(merge(&l, &r), m);
    }
    // Construct valid trees from plain parent maps; clocks choose a cross-side
    // cycle independently of the content timestamps.
    fn cycle_input(edges: &[(u128, u128, i64)], tombstone: bool) -> Database {
        let mut d = make_db(vec![]);
        let root = d.root.uuid;
        let mut pending: BTreeMap<_, _> = edges
            .iter()
            .map(|&(id, parent, time)| {
                let mut g = group(id);
                g.location_changed = Timestamp(time);
                g.entries.push(make_entry(id + 100, "retained", 1000));
                (id, (parent, g))
            })
            .collect();
        while !pending.is_empty() {
            let leaf = *pending
                .keys()
                .find(|id| !pending.values().any(|(p, _)| p == *id))
                .unwrap();
            let (parent, g) = pending.remove(&leaf).unwrap();
            if parent == 0 {
                d.root.groups.push(g);
            } else {
                pending.get_mut(&parent).unwrap().1.groups.push(g);
            }
        }
        if tombstone {
            d.deleted_objects.push(DeletedObject {
                uuid: Uuid::from_u128(2),
                deleted_at: Timestamp(999),
            });
        }
        assert_eq!(d.root.uuid, root);
        d
    }
    fn cycle_tree(d: &Database) -> BTreeMap<Uuid, Uuid> {
        let mut out = BTreeMap::new();
        let mut stack = vec![&d.root];
        while let Some(g) = stack.pop() {
            for child in &g.groups {
                assert!(out.insert(child.uuid, g.uuid).is_none());
                stack.push(child);
            }
            for e in &g.entries {
                assert!(out.insert(e.uuid, g.uuid).is_none());
            }
        }
        out
    }
    #[test]
    fn cleanup_cycle_cuts_oldest_move_not_smallest_uuid() {
        let l = cycle_input(&[(2, 3, 9), (3, 0, 1), (4, 2, 1)], false);
        let r = cycle_input(&[(2, 0, 1), (3, 2, 6), (4, 2, 1)], false);
        for m in [merge(&l, &r), merge(&r, &l)] {
            let tree = cycle_tree(&m);
            assert_eq!(tree[&Uuid::from_u128(3)], m.root.uuid);
            assert_eq!(tree[&Uuid::from_u128(2)], Uuid::from_u128(3));
            assert_eq!(tree[&Uuid::from_u128(4)], Uuid::from_u128(2));
        }
    }
    #[test]
    fn cross_move_cycle_keeps_descendants_and_entries() {
        for tombstone in [false, true] {
            let l = cycle_input(&[(2, 3, 5), (3, 0, 1), (4, 2, 1), (5, 4, 1)], tombstone);
            let r = cycle_input(&[(2, 0, 1), (3, 2, 6), (4, 2, 1), (5, 4, 1)], false);
            let expected = BTreeMap::from([
                (2, 100),
                (3, 2),
                (4, 2),
                (5, 4),
                (102, 2),
                (103, 3),
                (104, 4),
                (105, 5),
            ])
            .into_iter()
            .map(|(a, b)| (Uuid::from_u128(a), Uuid::from_u128(b)))
            .collect();
            for m in [merge(&l, &r), merge(&r, &l)] {
                assert_eq!(cycle_tree(&m), expected);
                assert!(m.deleted_objects.is_empty());
            }
        }
    }
    #[test]
    fn self_parent_cycle_detaches_only_itself() {
        // A unique-UUID value tree cannot encode a self-parent. Exercise the
        // functional-graph boundary directly, including an attached descendant.
        let a = Uuid::from_u128(2);
        let c = Uuid::from_u128(4);
        let parents = BTreeMap::from([(a, a), (c, a)]);
        assert_eq!(
            cycle_roots(&parents, &HashSet::new(), &HashMap::new()),
            vec![a]
        );
    }
    #[test]
    fn tombstoned_cycle_member_leaves_live_descendants() {
        let mut l = cycle_input(&[(2, 3, 5), (3, 0, 1), (4, 2, 1), (5, 4, 1)], false);
        let r = cycle_input(&[(2, 0, 1), (3, 2, 6), (4, 2, 1), (5, 4, 1)], false);
        l.deleted_objects.push(DeletedObject {
            uuid: Uuid::from_u128(2),
            deleted_at: Timestamp(2000),
        });
        for m in [merge(&l, &r), merge(&r, &l)] {
            let tree = cycle_tree(&m);
            assert_eq!(tree.len(), 7);
            for (id, parent) in [
                (3, 100),
                (4, 100),
                (5, 4),
                (102, 100),
                (103, 3),
                (104, 4),
                (105, 5),
            ] {
                assert_eq!(tree[&Uuid::from_u128(id)], Uuid::from_u128(parent));
            }
        }
    }
    #[test]
    fn resumed_cycle_bfs_obeys_depth_cap() {
        let mut left = vec![(2, 3, 5), (3, 0, 1)];
        let mut right = vec![(2, 0, 1), (3, 2, 6)];
        for id in 4..100 {
            let edge = (id, if id == 4 { 2 } else { id - 1 }, 1);
            left.push(edge);
            right.push(edge);
        }
        let l = cycle_input(&left, false);
        let r = cycle_input(&right, false);
        for m in [merge(&l, &r), merge(&r, &l)] {
            assert_eq!(cycle_tree(&m).len(), 196);
            let mut stack = vec![(&m.root, 1)];
            while let Some((g, depth)) = stack.pop() {
                assert!(depth <= MAX_GROUP_DEPTH);
                stack.extend(g.groups.iter().map(|g| (g, depth + 1)));
            }
            let password = secrecy::SecretString::from("test-only");
            assert_eq!(
                Database::open(&m.save(&password).unwrap(), &password).unwrap(),
                m
            );
        }
    }
    #[test]
    fn three_group_cycle_keeps_descendants() {
        let l = cycle_input(&[(2, 3, 5), (3, 4, 5), (4, 0, 1), (5, 2, 1)], false);
        let r = cycle_input(&[(2, 0, 1), (3, 0, 1), (4, 2, 6), (5, 2, 1)], false);
        for m in [merge(&l, &r), merge(&r, &l)] {
            let tree = cycle_tree(&m);
            assert_eq!(tree.len(), 8);
            for (id, parent) in [(2, 100), (3, 4), (4, 2), (5, 2)] {
                assert_eq!(tree[&Uuid::from_u128(id)], Uuid::from_u128(parent));
                assert_eq!(tree[&Uuid::from_u128(id + 100)], Uuid::from_u128(id));
            }
        }
    }

    #[test]
    fn orphan_group_and_entries_survive_deleted_parent() {
        let mut l = make_db(vec![]);
        let mut r = l.clone();
        let mut a = group(2);
        a.groups.push(group(3));
        r.root.groups.push(a);
        l.deleted_objects.push(DeletedObject {
            uuid: Uuid::from_u128(2),
            deleted_at: Timestamp(2000),
        });
        let m = merge(&l, &r);
        assert_eq!(m.root.groups.len(), 1);
        assert_eq!(m.root.groups[0].uuid, Uuid::from_u128(3));
    }
    #[test]
    fn large_vault_has_exactly_one_sorted_copy_of_each_entry() {
        let l = make_db(
            (1..=20_000)
                .rev()
                .map(|id| make_entry(id + 1000, "entry", 1000))
                .collect(),
        );
        let m = merge(&l, &l);
        assert_eq!(m.root.entries.len(), 20_000);
        assert!(m.root.entries.windows(2).all(|w| w[0].uuid < w[1].uuid));
    }
    #[test]
    fn deep_caller_tree_is_flattened_to_serializable_depth() {
        let mut g = group(2000);
        for id in (2001..4000).rev() {
            let mut p = group(id);
            p.groups.push(g);
            g = p;
        }
        let mut l = make_db(vec![]);
        l.root.groups.push(g);
        let m = merge(&l, &make_db(vec![]));
        let mut count = 0;
        let mut stack = vec![(&m.root, 1)];
        while let Some((g, depth)) = stack.pop() {
            assert!(depth <= MAX_GROUP_DEPTH);
            count += 1;
            stack.extend(g.groups.iter().map(|g| (g, depth + 1)));
        }
        assert_eq!(count, 2001);
        let password = secrecy::SecretString::from("test-only");
        let reopened = Database::open(&m.save(&password).unwrap(), &password).unwrap();
        assert_eq!(reopened, m);
        // Iteratively dismantle the deliberately hostile input as Group's derived Drop is recursive.
        let mut stack = std::mem::take(&mut l.root.groups);
        while let Some(mut g) = stack.pop() {
            stack.append(&mut g.groups);
        }
    }
    #[test]
    fn protected_field_content_is_part_of_history_identity() {
        let (mut l, mut r) = pair(2000, 2000);
        l.root.entries[0].set_field("Password", "one", true);
        r.root.entries[0].set_field("Password", "two", true);
        let m = merge(&l, &r);
        assert_eq!(m.root.entries[0].history[0].field("Password"), Some("two"));
    }
    #[test]
    fn local_settings_and_meta_are_preserved() {
        let (mut l, mut r) = pair(2000, 3000);
        l.meta.database_description = "local".into();
        r.meta.database_description = "remote".into();
        r.settings.minor_version = 1;
        let m = merge(&l, &r);
        assert_eq!(m.meta, l.meta);
        assert_eq!(m.settings, l.settings);
    }
    #[test]
    fn resurrection_uses_newest_surviving_version() {
        let (mut l, r) = pair(1000, 3000);
        l.deleted_objects.push(DeletedObject {
            uuid: Uuid::from_u128(1),
            deleted_at: Timestamp(2000),
        });
        let m = merge(&l, &r);
        assert!(m.deleted_objects.is_empty());
        assert_eq!(m.root.entries.len(), 1);
    }
    #[test]
    fn uuid_collision_between_entry_and_group_keeps_first_occurrence() {
        let mut l = make_db(vec![make_entry(2, "first", 1000)]);
        let mut duplicate = group(2);
        duplicate.entries.push(make_entry(3, "child", 1000));
        l.root.groups.push(duplicate);
        let m = merge(&l, &make_db(vec![]));
        assert!(m.root.groups.is_empty());
        assert_eq!(
            m.root.entries.iter().map(|e| e.uuid).collect::<Vec<_>>(),
            vec![Uuid::from_u128(2), Uuid::from_u128(3)]
        );
    }
}
