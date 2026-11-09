//! First-use attachment pool, keyed by exact bytes and protection flag.
use crate::format::inner_header::InnerBinary;
use crate::{Attachment, Entry, Group};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
type PoolKey = ([u8; 32], bool);
fn attachment_key(a: &Attachment) -> PoolKey {
    (Sha256::digest(&a.data).into(), a.protected)
}
pub(crate) struct AttachmentPool {
    pub binaries: Vec<InnerBinary>,
    indices: HashMap<PoolKey, Vec<usize>>,
}
impl AttachmentPool {
    pub(crate) fn collecting(root: &Group) -> Self {
        let mut pool = Self {
            binaries: vec![],
            indices: HashMap::new(),
        };
        pool.group(root);
        pool
    }
    fn group(&mut self, g: &Group) {
        for e in &g.entries {
            self.entry(e);
        }
        for g in &g.groups {
            self.group(g);
        }
    }
    fn entry(&mut self, e: &Entry) {
        for a in &e.attachments {
            let key = attachment_key(a);
            if self.find_index(&key, a).is_none() {
                let index = self.binaries.len();
                self.binaries.push(InnerBinary {
                    data: a.data.clone(),
                    protected: a.protected,
                });
                self.indices.entry(key).or_default().push(index);
            }
        }
        for e in &e.history {
            self.entry(e);
        }
    }
    fn find_index(&self, key: &PoolKey, a: &Attachment) -> Option<usize> {
        // Compare exact content within a digest bucket: even a hash collision cannot merge data.
        self.indices.get(key)?.iter().copied().find(|index| {
            self.binaries
                .get(*index)
                .is_some_and(|b| b.data == a.data && b.protected == a.protected)
        })
    }
    pub(crate) fn index_of(&self, a: &Attachment) -> usize {
        // Only called for the same immutable tree passed to collecting. Expected O(1) pool
        // lookup after hashing the attachment, independent of the number of other binaries.
        self.find_index(&attachment_key(a), a)
            .unwrap_or(self.binaries.len())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Timestamp;
    fn make_attachment(data: &[u8], protected: bool) -> Attachment {
        Attachment {
            name: "file".into(),
            data: data.to_vec(),
            protected,
        }
    }
    fn make_root(attachments: Vec<Attachment>) -> Group {
        let mut g = Group::new("g", Timestamp(0));
        let mut e = Entry::new(Timestamp(0));
        e.attachments = attachments;
        g.entries.push(e);
        g
    }
    #[test]
    fn digest_collision_still_requires_exact_content() {
        let a = make_attachment(&[1], false);
        let b = make_attachment(&[2], false);
        let mut pool = AttachmentPool::collecting(&make_root(vec![a.clone(), b.clone()]));
        let key = attachment_key(&a);
        // Force both distinct contents into one bucket, simulating a digest collision.
        pool.indices.insert(key, vec![0, 1]);
        assert_eq!(pool.find_index(&key, &a), Some(0));
        assert_eq!(pool.find_index(&key, &b), Some(1));
        assert_eq!(pool.find_index(&key, &make_attachment(&[3], false)), None);
    }
    #[test]
    fn many_attachments_deduplicate_in_first_use_order() {
        let originals: Vec<_> = (0..2000u32)
            .map(|n| make_attachment(&n.to_le_bytes(), n % 2 == 0))
            .collect();
        let mut all = originals.clone();
        all.extend(originals.iter().rev().cloned());
        let pool = AttachmentPool::collecting(&make_root(all));
        assert_eq!(pool.binaries.len(), 2000);
        for (i, a) in originals.iter().enumerate() {
            assert_eq!(pool.index_of(a), i);
            assert_eq!(pool.binaries[i].data, a.data);
            assert_eq!(pool.binaries[i].protected, a.protected);
        }
    }
    #[test]
    fn identical_content_and_flag_stored_once() {
        let a = make_attachment(&[1, 2], true);
        let mut b = a.clone();
        b.name = "other".into();
        let pool = AttachmentPool::collecting(&make_root(vec![a.clone(), b.clone()]));
        assert_eq!(pool.binaries.len(), 1);
        assert_eq!(pool.index_of(&a), 0);
        assert_eq!(pool.index_of(&b), 0);
    }
    #[test]
    fn same_content_different_protection_stored_twice() {
        let a = make_attachment(&[1, 2], true);
        let b = make_attachment(&[1, 2], false);
        let pool = AttachmentPool::collecting(&make_root(vec![a.clone(), b.clone()]));
        assert_eq!(pool.binaries.len(), 2);
        assert_eq!(pool.index_of(&a), 0);
        assert_eq!(pool.index_of(&b), 1);
        assert!(!pool.binaries[1].protected);
    }
    #[test]
    fn history_only_attachment_included() {
        let mut g = make_root(vec![]);
        let mut h = Entry::new(Timestamp(0));
        h.attachments.push(make_attachment(&[9], false));
        g.entries[0].history.push(h);
        assert_eq!(AttachmentPool::collecting(&g).binaries[0].data, vec![9]);
    }
    #[test]
    fn order_is_first_use_in_encoder_traversal() {
        let mut g = make_root(vec![make_attachment(&[0], false)]);
        let mut h = Entry::new(Timestamp(0));
        h.attachments.push(make_attachment(&[1], false));
        g.entries[0].history.push(h);
        g.entries
            .extend(make_root(vec![make_attachment(&[2], false)]).entries);
        g.groups.push(make_root(vec![make_attachment(&[3], false)]));
        assert_eq!(
            AttachmentPool::collecting(&g)
                .binaries
                .iter()
                .map(|b| b.data.clone())
                .collect::<Vec<_>>(),
            vec![vec![0], vec![1], vec![2], vec![3]]
        );
    }
    #[test]
    fn nested_history_attachment_included() {
        let mut g = make_root(vec![]);
        let mut h = Entry::new(Timestamp(0));
        h.history
            .extend(make_root(vec![make_attachment(&[7], true)]).entries);
        g.entries[0].history.push(h);
        assert_eq!(AttachmentPool::collecting(&g).binaries[0].data, vec![7]);
    }
    #[test]
    fn no_attachments_yields_empty_pool() {
        assert!(AttachmentPool::collecting(&make_root(vec![]))
            .binaries
            .is_empty());
    }
    #[test]
    fn empty_data_attachment_kept() {
        let p = AttachmentPool::collecting(&make_root(vec![make_attachment(&[], false)]));
        assert_eq!(p.binaries.len(), 1);
        assert!(p.binaries[0].data.is_empty());
    }
}
