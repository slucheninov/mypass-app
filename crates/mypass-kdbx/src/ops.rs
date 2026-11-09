//! Database mutations and history management. Tree walks are iterative, including mutable
//! lookups: callers may also assemble trees through the raw model accessors.
use crate::*;
use std::collections::HashSet;
use uuid::Uuid;

fn invalid(message: &str) -> KdbxError {
    KdbxError::InvalidOperation(message.into())
}
fn groups(root: &Group) -> impl Iterator<Item = (&Group, usize)> {
    let mut stack = vec![(root, 1usize)];
    std::iter::from_fn(move || {
        let (g, depth) = stack.pop()?;
        stack.extend(g.groups.iter().rev().map(|g| (g, depth.saturating_add(1))));
        Some((g, depth))
    })
}
fn snapshot_size(e: &Entry) -> u128 {
    e.fields
        .iter()
        .fold(0u128, |n, f| {
            n.saturating_add(f.key.len() as u128)
                .saturating_add(f.value.len() as u128)
        })
        .saturating_add(e.attachments.iter().fold(0u128, |n, a| {
            n.saturating_add(a.data.len() as u128)
                .saturating_add(a.name.len() as u128)
        }))
}
fn trim_history(e: &mut Entry, items: i32, size: i64) {
    if items >= 0 {
        let excess = e.history.len().saturating_sub(items as usize);
        e.history.drain(..excess);
    }
    if size >= 0 {
        let mut total = e
            .history
            .iter()
            .fold(0u128, |n, h| n.saturating_add(snapshot_size(h)));
        let mut count = 0;
        for h in &e.history {
            if total <= size as u128 {
                break;
            }
            total = total.saturating_sub(snapshot_size(h));
            count += 1;
        }
        e.history.drain(..count);
    }
}
impl Database {
    /// Returns the root group's identity.
    pub fn root_id(&self) -> Uuid {
        self.root.uuid
    }
    /// Returns an entry, or None when absent. History snapshots are not searched.
    pub fn find_entry(&self, id: Uuid) -> Option<&Entry> {
        groups(&self.root).find_map(|(g, _)| g.entries.iter().find(|e| e.uuid == id))
    }
    /// Raw mutable entry lookup; absent entries return None. No time/history handling.
    pub fn find_entry_mut(&mut self, id: Uuid) -> Option<&mut Entry> {
        let mut stack = vec![&mut self.root];
        while let Some(g) = stack.pop() {
            if let Some(e) = g.entries.iter_mut().find(|e| e.uuid == id) {
                return Some(e);
            }
            stack.extend(g.groups.iter_mut().rev());
        }
        None
    }
    /// Returns a group, including the root, or None when absent.
    pub fn find_group(&self, id: Uuid) -> Option<&Group> {
        groups(&self.root).find_map(|(g, _)| (g.uuid == id).then_some(g))
    }
    /// Raw mutable group lookup; returns None when absent.
    pub fn find_group_mut(&mut self, id: Uuid) -> Option<&mut Group> {
        let mut stack = vec![&mut self.root];
        while let Some(g) = stack.pop() {
            if g.uuid == id {
                return Some(g);
            }
            stack.extend(g.groups.iter_mut().rev());
        }
        None
    }
    /// Returns the containing group, or None for an absent entry.
    pub fn parent_of_entry(&self, id: Uuid) -> Option<Uuid> {
        groups(&self.root)
            .find_map(|(g, _)| g.entries.iter().any(|e| e.uuid == id).then_some(g.uuid))
    }
    /// Records access only, saturating usage count; false for an absent entry.
    pub fn mark_accessed(&mut self, id: Uuid, now: Timestamp) -> bool {
        if let Some(e) = self.find_entry_mut(id) {
            e.accessed = now;
            e.usage_count = e.usage_count.saturating_add(1);
            true
        } else {
            false
        }
    }
    /// True for entries anywhere below the configured, existing recycle bin.
    pub fn is_in_recycle_bin(&self, entry_id: Uuid) -> bool {
        self.meta
            .recycle_bin_uuid
            .and_then(|id| self.find_group(id))
            .is_some_and(|bin| {
                groups(bin).any(|(g, _)| g.entries.iter().any(|e| e.uuid == entry_id))
            })
    }
    /// Walks entries in document order, pruning each listed group and its entire subtree.
    pub fn entries_excluding(&self, skip_groups: &[Uuid]) -> Vec<&Entry> {
        let mut stack = vec![&self.root];
        let mut entries = vec![];
        while let Some(g) = stack.pop() {
            if skip_groups.contains(&g.uuid) {
                continue;
            }
            entries.extend(&g.entries);
            stack.extend(g.groups.iter().rev());
        }
        entries
    }
    /// Adds an entry; rejects a missing parent or an identity already in the live tree.
    pub fn add_entry(&mut self, parent: Uuid, mut entry: Entry, now: Timestamp) -> Result<()> {
        if self.id_exists(entry.uuid) {
            return Err(invalid("duplicate UUID"));
        }
        let g = self
            .find_group_mut(parent)
            .ok_or_else(|| invalid("missing parent"))?;
        entry.location_changed = now;
        g.entries.push(entry);
        Ok(())
    }
    /// Edits content and records its previous version; rejects a missing entry.
    /// Identity and history are preserved regardless of edits to them by the closure.
    pub fn update_entry(
        &mut self,
        id: Uuid,
        now: Timestamp,
        edit: impl FnOnce(&mut Entry),
    ) -> Result<bool> {
        let (items, size) = (self.meta.history_max_items, self.meta.history_max_size);
        let e = self
            .find_entry_mut(id)
            .ok_or_else(|| invalid("missing entry"))?;
        let mut before = e.clone();
        edit(e);
        e.uuid = before.uuid;
        e.history = before.history.clone();
        if before.content_eq(e) {
            *e = before;
            return Ok(false);
        }
        before.history.clear();
        e.history.push(before);
        e.modified = now;
        trim_history(e, items, size);
        Ok(true)
    }
    /// Moves an entry without editing content; rejects missing entry or destination.
    pub fn move_entry(&mut self, id: Uuid, new_parent: Uuid, now: Timestamp) -> Result<()> {
        if self.find_group(new_parent).is_none() {
            return Err(invalid("missing parent"));
        }
        let mut e = self.take_entry(id)?;
        e.location_changed = now;
        self.find_group_mut(new_parent)
            .ok_or_else(|| invalid("missing parent"))?
            .entries
            .push(e);
        Ok(())
    }
    /// Recycles an entry, or purges when disabled/already recycled; rejects missing entry.
    pub fn delete_entry(&mut self, id: Uuid, now: Timestamp) -> Result<()> {
        if self.find_entry(id).is_none() {
            return Err(invalid("missing entry"));
        }
        if !self.meta.recycle_bin_enabled || self.is_in_recycle_bin(id) {
            return self.purge_entry(id, now);
        }
        let bin = self.ensure_bin(now)?;
        self.move_entry(id, bin, now)
    }
    /// Moves an entry to a target group; rejects missing entry or target.
    pub fn restore_entry(&mut self, id: Uuid, target_group: Uuid, now: Timestamp) -> Result<()> {
        self.move_entry(id, target_group, now)
    }
    /// Permanently removes an entry and tombstones its UUID once; rejects missing entry.
    pub fn purge_entry(&mut self, id: Uuid, now: Timestamp) -> Result<()> {
        self.take_entry(id)?;
        self.tombstone(id, now);
        Ok(())
    }
    /// Adds a subtree; rejects missing parent, duplicate live UUIDs or excessive group depth.
    pub fn add_group(&mut self, parent: Uuid, mut group: Group, now: Timestamp) -> Result<()> {
        self.check_depth(parent, &group)?;
        let mut ids = HashSet::new();
        for (g, _) in groups(&group) {
            for id in std::iter::once(g.uuid).chain(g.entries.iter().map(|e| e.uuid)) {
                if !ids.insert(id) || self.id_exists(id) {
                    return Err(invalid("duplicate UUID"));
                }
            }
        }
        group.location_changed = now;
        self.find_group_mut(parent)
            .ok_or_else(|| invalid("missing parent"))?
            .groups
            .push(group);
        Ok(())
    }
    /// Moves a subtree; rejects root, missing groups, cycles or excessive resulting depth.
    pub fn move_group(&mut self, id: Uuid, new_parent: Uuid, now: Timestamp) -> Result<()> {
        if id == self.root_id() {
            return Err(invalid("cannot move root"));
        }
        let g = self
            .find_group(id)
            .ok_or_else(|| invalid("missing group"))?;
        if groups(g).any(|(g, _)| g.uuid == new_parent) {
            return Err(invalid("group cycle"));
        }
        self.check_depth(new_parent, g)?;
        let mut g = self.take_group(id)?;
        g.location_changed = now;
        self.find_group_mut(new_parent)
            .ok_or_else(|| invalid("missing parent"))?
            .groups
            .push(g);
        Ok(())
    }
    /// Recycles a group or purges it; rejects root, the bin itself, missing groups or depth overflow.
    pub fn delete_group(&mut self, id: Uuid, now: Timestamp) -> Result<()> {
        self.check_group_delete(id)?;
        let in_bin = self
            .meta
            .recycle_bin_uuid
            .and_then(|bin| self.find_group(bin))
            .is_some_and(|bin| groups(bin).any(|(g, _)| g.uuid == id));
        if !self.meta.recycle_bin_enabled || in_bin {
            return self.purge_group(id, now);
        }
        // Validate before creating a bin so a failed depth check leaves the database untouched.
        let g = self
            .find_group(id)
            .ok_or_else(|| invalid("missing group"))?;
        if let Some(bin) = self
            .meta
            .recycle_bin_uuid
            .filter(|bin| self.find_group(*bin).is_some())
        {
            self.check_depth(bin, g)?;
        } else if groups(g)
            .map(|(_, d)| d)
            .max()
            .unwrap_or(0)
            .saturating_add(2)
            > crate::format::limits::MAX_GROUP_DEPTH
        {
            return Err(invalid("group depth"));
        }
        let bin = self.ensure_bin(now)?;
        self.move_group(id, bin, now)
    }
    /// Permanently removes a subtree, tombstoning live groups/entries only; rejects root/bin/missing.
    pub fn purge_group(&mut self, id: Uuid, now: Timestamp) -> Result<()> {
        self.check_group_delete(id)?;
        let g = self.take_group(id)?;
        for (g, _) in groups(&g) {
            self.tombstone(g.uuid, now);
            for e in &g.entries {
                self.tombstone(e.uuid, now);
            }
        }
        Ok(())
    }
    fn id_exists(&self, id: Uuid) -> bool {
        self.find_group(id).is_some() || self.find_entry(id).is_some()
    }
    fn check_depth(&self, parent: Uuid, subtree: &Group) -> Result<()> {
        let depth = groups(&self.root)
            .find_map(|(g, d)| (g.uuid == parent).then_some(d))
            .ok_or_else(|| invalid("missing parent"))?;
        let height = groups(subtree).map(|(_, d)| d).max().unwrap_or(0);
        if depth.saturating_add(height) > crate::format::limits::MAX_GROUP_DEPTH {
            return Err(invalid("group depth"));
        }
        Ok(())
    }
    fn take_entry(&mut self, id: Uuid) -> Result<Entry> {
        let parent = self
            .parent_of_entry(id)
            .ok_or_else(|| invalid("missing entry"))?;
        let g = self
            .find_group_mut(parent)
            .ok_or_else(|| invalid("missing parent"))?;
        let pos = g
            .entries
            .iter()
            .position(|e| e.uuid == id)
            .ok_or_else(|| invalid("missing entry"))?;
        Ok(g.entries.remove(pos))
    }
    fn take_group(&mut self, id: Uuid) -> Result<Group> {
        let parent = groups(&self.root)
            .find_map(|(g, _)| g.groups.iter().any(|c| c.uuid == id).then_some(g.uuid))
            .ok_or_else(|| invalid("missing group or root"))?;
        let g = self
            .find_group_mut(parent)
            .ok_or_else(|| invalid("missing parent"))?;
        let pos = g
            .groups
            .iter()
            .position(|g| g.uuid == id)
            .ok_or_else(|| invalid("missing group"))?;
        Ok(g.groups.remove(pos))
    }
    fn check_group_delete(&self, id: Uuid) -> Result<()> {
        if id == self.root_id() || self.meta.recycle_bin_uuid == Some(id) {
            return Err(invalid("cannot delete root or recycle bin"));
        }
        self.find_group(id)
            .ok_or_else(|| invalid("missing group"))?;
        Ok(())
    }
    fn tombstone(&mut self, id: Uuid, now: Timestamp) {
        if !self.deleted_objects.iter().any(|d| d.uuid == id) {
            self.deleted_objects.push(DeletedObject {
                uuid: id,
                deleted_at: now,
            });
        }
    }
    fn ensure_bin(&mut self, now: Timestamp) -> Result<Uuid> {
        if let Some(id) = self
            .meta
            .recycle_bin_uuid
            .filter(|id| self.find_group(*id).is_some())
        {
            return Ok(id);
        }
        // This Result-returning path uses fallible OS randomness, like Database::save.
        let mut bytes = [0; 16];
        crate::format::kdf::fill_random(&mut bytes)?;
        let id = uuid::Builder::from_random_bytes(bytes).into_uuid();
        let bin = Group {
            uuid: id,
            name: "Recycle Bin".into(),
            icon_id: 43,
            created: now,
            modified: now,
            accessed: now,
            location_changed: now,
            expires: false,
            expiry_time: None,
            entries: vec![],
            groups: vec![],
            custom_data: vec![],
            extra: vec![],
        };
        self.add_group(self.root_id(), bin, now)?;
        self.meta.recycle_bin_uuid = Some(id);
        Ok(id)
    }
}
#[cfg(test)]
mod tests;
