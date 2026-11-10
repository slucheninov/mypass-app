// Test-only support code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use mypass_core::{matcher::credential_candidates, merge::merge};
use mypass_kdbx::{Database, Entry, KdfParams, Timestamp};
use secrecy::SecretString;
#[test]
fn merge_of_two_distinct_recycle_bins_keeps_recycled_entries_out_of_candidates() {
    let mut a = Database::new("synthetic");
    a.settings.kdf = KdfParams::AesKdf {
        seed: vec![0; 32],
        rounds: 1,
    };
    let mut ids = vec![];
    for title in ["Victim", "Old"] {
        let mut e = Entry::new(Timestamp(1));
        e.set_field("Title", title, false);
        e.set_field("URL", "https://example.com", false);
        e.set_field("Password", "synthetic", true);
        ids.push(e.uuid);
        a.root.entries.push(e);
    }
    let mut b = a.clone();
    a.delete_entry(ids[0], Timestamp(2)).unwrap();
    b.delete_entry(ids[1], Timestamp(3)).unwrap();
    let merged = merge(&a, &b);
    let password = SecretString::from("synthetic");
    let reopened = Database::open(&merged.save(&password).unwrap(), &password).unwrap();
    assert!(credential_candidates(&reopened, "https://example.com/x").is_empty());
    for id in ids {
        assert!(reopened.find_entry(id).is_some());
    }
    assert_eq!(merge(&a, &merged), merged);
}
