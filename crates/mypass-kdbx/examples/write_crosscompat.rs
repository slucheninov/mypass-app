// Test-only support code: synthetic interoperability fixture, never an owner's vault.

use mypass_kdbx::{Attachment, Database, Entry, Timestamp};
use secrecy::SecretString;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: write_crosscompat <out.kdbx>")?;
    let mut db = Database::new("Cross-implementation fixture");
    let mut entry = Entry::new(Timestamp(1_700_000_000));
    entry.set_field("Title", "Round Trip Entry", false);
    entry.set_field("UserName", "roundtrip-user", false);
    entry.set_field("Password", "roundtrip-pass", true);
    entry.history.push(entry.clone());
    entry.attachments.push(Attachment {
        name: "data.bin".into(),
        data: (0..=255).collect(),
        protected: false,
    });
    db.root.entries.push(entry);
    let mut unicode = Entry::new(Timestamp(1_700_000_000));
    unicode.set_field("Title", "\u{411}\u{430}\u{43d}\u{43a}", false);
    unicode.set_field("UserName", "", false);
    unicode.set_field(
        "Password",
        "\u{43f}\u{430}\u{440}\u{43e}\u{43b}\u{44c}-1",
        true,
    );
    db.root.entries.push(unicode);
    // Database::new selects AES-256-CBC, gzip and Argon2id (64 MiB, 3 passes, 2 lanes).
    use std::io::Write;
    let bytes = db.save(&SecretString::from("RoundTripPass123"))?;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(&bytes)?;
    Ok(())
}
