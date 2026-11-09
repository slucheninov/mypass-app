// Test-only support code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use mypass_kdbx::{Database, Group};
use secrecy::SecretString;

fn entries(group: &Group) -> Vec<&mypass_kdbx::Entry> {
    group
        .entries
        .iter()
        .chain(group.groups.iter().flat_map(entries))
        .collect()
}

#[test]
fn opens_vault_generated_by_pykeepass() {
    let Some(path) = std::env::var_os("MYPASS_PYKEEPASS_VAULT") else {
        println!("skipped: MYPASS_PYKEEPASS_VAULT is not set");
        return;
    };
    let bytes = std::fs::read(path).expect("read pykeepass fixture");
    let db = Database::open(&bytes, &SecretString::from("fixture-password"))
        .expect("open pykeepass fixture");
    let all = entries(&db.root);
    assert_eq!(all.len(), 2);
    let entry = all
        .iter()
        .find(|e| e.title() == "Made By Pykeepass")
        .unwrap();
    assert_eq!(entry.field("UserName"), Some("pk-user"));
    assert_eq!(entry.field("Password"), Some("pk-pass"));
    let unicode = all
        .iter()
        .find(|e| e.title() == "\u{411}\u{430}\u{43d}\u{43a}")
        .unwrap();
    assert_eq!(unicode.field("UserName"), Some("unicode-user"));
    assert_eq!(
        unicode.field("Password"),
        Some("\u{43f}\u{430}\u{440}\u{43e}\u{43b}\u{44c}-1")
    );
}
