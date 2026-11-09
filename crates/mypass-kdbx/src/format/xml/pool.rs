//! First-use attachment pool, keyed by exact bytes and protection flag.
use crate::format::inner_header::InnerBinary;
use crate::{Attachment, Entry, Group};
pub(crate) struct AttachmentPool {
    pub binaries: Vec<InnerBinary>,
}
impl AttachmentPool {
    pub(crate) fn collecting(root: &Group) -> Self {
        let mut pool = Self { binaries: vec![] };
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
            if !self
                .binaries
                .iter()
                .any(|b| b.data == a.data && b.protected == a.protected)
            {
                self.binaries.push(InnerBinary {
                    data: a.data.clone(),
                    protected: a.protected,
                });
            }
        }
        for e in &e.history {
            self.entry(e);
        }
    }
    pub(crate) fn index_of(&self, a: &Attachment) -> usize {
        // Only called for the same immutable tree passed to collecting.
        self.binaries
            .iter()
            .position(|b| b.data == a.data && b.protected == a.protected)
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
