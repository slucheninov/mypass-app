//! End-to-end journeys through the `AppCore` facade over real temp files, a manual clock and the
//! fake platform: what the UI's commands do, in the order a user does them.

// Test-only support code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use keepass::DatabaseKey;
use mypass_kdbx::{Entry, FileIo, PlainFileIo, Timestamp, VaultFile};
use mypass_platform::testing::{fake_platform, FakePlatform};
use mypass_platform::AppPaths;
use mypass_session::dto::{
    AddressDto, CardEdit, Category, EntryEditModel, EntryQuery, EntryTypeDto, FieldRef, Scope,
    SortKey,
};
use mypass_session::editing::new_model;
use mypass_session::saving::ExternalChange;
use mypass_session::scheduler::ManualScheduler;
use mypass_session::settings::{SettingsPatch, SettingsStore};
use mypass_session::testing::fast_database;
use mypass_session::{AppCore, AttachmentOp, GenerateRequest, GeneratorOptionsDto};
use secrecy::SecretString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};
use tempfile::TempDir;
use uuid::Uuid;

const T0: u64 = 1_700_000_000;
const SAVE: Duration = Duration::from_secs(1);

fn pw(s: &str) -> SecretString {
    SecretString::from(s.to_owned())
}

struct World {
    dir: TempDir,
    paths: AppPaths,
    clock: Arc<ManualScheduler>,
    core: AppCore,
    path: PathBuf,
}

fn core_over(paths: &AppPaths) -> (AppCore, Arc<ManualScheduler>, FakePlatform) {
    let fake = fake_platform();
    let clock = ManualScheduler::new(UNIX_EPOCH + Duration::from_secs(T0));
    let core = AppCore::with_parts(
        paths.clone(),
        fake.platform.clone(),
        clock.clone(),
        SettingsStore::in_memory(),
    );
    (core, clock, fake)
}

/// A new, unlocked, empty vault `vault.kdbx` (password `pw`, cheap KDF).
fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let paths = AppPaths::macos(dir.path());
    let (core, clock, _fake) = core_over(&paths);
    let path = dir.path().join("vault.kdbx");
    core.session
        .create_vault_with(&path, pw("pw"), fast_database("Journey"))
        .unwrap();
    World {
        dir,
        paths,
        clock,
        core,
        path,
    }
}

fn root(core: &AppCore) -> Uuid {
    Uuid::parse_str(&core.sidebar_get().unwrap().root_group_id).unwrap()
}

fn login(core: &AppCore, title: &str, password: &str) -> String {
    let mut m = new_model(EntryTypeDto::Login, root(core));
    m.title = title.into();
    m.username = "user".into();
    m.password = password.into();
    core.entry_save(m).unwrap()
}

fn all(core: &AppCore, search: &str) -> Vec<String> {
    let q = EntryQuery {
        scope: Scope::Category {
            category: Category::All,
        },
        search: search.into(),
        sort: SortKey::Title,
    };
    core.entry_list(q)
        .unwrap()
        .into_iter()
        .map(|e| e.title)
        .collect()
}

fn model_of(core: &AppCore, kind: EntryTypeDto, title: &str) -> EntryEditModel {
    let mut m = new_model(kind, root(core));
    m.title = title.into();
    m
}

#[test]
fn full_lifecycle_create_edit_lock_unlock_reopen() {
    let w = world();
    let attachment = w.dir.path().join("note.txt");
    std::fs::write(&attachment, b"attached bytes").unwrap();

    let mut m = model_of(&w.core, EntryTypeDto::Login, "Login");
    m.password = "login-pw".into();
    w.core.entry_save(m).unwrap();
    let mut m = model_of(&w.core, EntryTypeDto::Address, "Address");
    m.address = Some(AddressDto {
        label: "Home".into(),
        city: "Kyiv".into(),
        ..AddressDto::default()
    });
    w.core.entry_save(m).unwrap();
    let mut m = model_of(&w.core, EntryTypeDto::Card, "Card");
    m.card = Some(CardEdit {
        holder: "A Holder".into(),
        number: "4111111111111111".into(),
        expiry: "12/30".into(),
        cvv: "123".into(),
    });
    let card_id = w.core.entry_save(m).unwrap();
    let mut m = model_of(&w.core, EntryTypeDto::Notes, "Notes");
    m.notes = "a private note".into();
    w.core.entry_save(m).unwrap();
    let mut m = model_of(&w.core, EntryTypeDto::Files, "Files");
    m.attachment_ops = vec![AttachmentOp::Add {
        path: attachment.to_string_lossy().into_owned(),
    }];
    let files_id = w.core.entry_save(m).unwrap();

    w.clock.advance(SAVE);
    w.core.vault_lock().unwrap();

    // A new process: new core over the same paths, nothing shared but the files.
    let (second, _clock, _fake) = core_over(&w.paths);
    let status = second
        .vault_open(w.path.to_str().unwrap(), pw("pw"))
        .unwrap();
    assert_eq!(
        status.state,
        mypass_session::session::SessionState::Unlocked
    );
    assert_eq!(
        all(&second, ""),
        ["Address", "Card", "Files", "Login", "Notes"]
    );
    assert_eq!(
        second.entry_reveal(&card_id, FieldRef::CardNumber).unwrap(),
        "4111111111111111"
    );
    let files = second.entry_get(&files_id).unwrap();
    assert_eq!(files.attachments.len(), 1);
    assert_eq!(files.attachments[0].name, "note.txt");
    let out = w.dir.path().join("exported.txt");
    second
        .attachment_export(&files_id, "note.txt", out.to_str().unwrap())
        .unwrap();
    assert_eq!(std::fs::read(out).unwrap(), b"attached bytes");
}

#[test]
fn journey_external_keepassxc_style_edit_is_merged() {
    let w = world();
    let a = login(&w.core, "Alpha", "pa");
    let b = login(&w.core, "Beta", "pb");
    w.clock.advance(SAVE);

    // Another client renames Alpha through its own handle and saves.
    let (mut file, mut db) = VaultFile::open(&w.path, &pw("pw")).unwrap();
    let alpha = Uuid::parse_str(&a).unwrap();
    db.update_entry(alpha, Timestamp(T0 as i64 + 100), |e: &mut Entry| {
        e.set_field("Title", "Alpha (external)", false)
    })
    .unwrap();
    file.save(&db, &pw("pw")).unwrap();

    // Meanwhile a local edit of Beta is still unsaved.
    w.clock.advance(Duration::from_secs(200));
    let mut m = w.core.entry_edit_model(&b).unwrap();
    m.title = "Beta (local)".into();
    w.core.entry_save(m).unwrap();

    assert_eq!(
        w.core.session.check_external_change().unwrap(),
        ExternalChange::Merged
    );
    assert_eq!(
        all(&w.core, ""),
        ["Alpha (external)", "Beta (local)"],
        "both edits survive in the live session"
    );
    w.core.session.flush().unwrap();
    let on_disk = VaultFile::open(&w.path, &pw("pw")).unwrap().1;
    let mut titles: Vec<&str> = on_disk.root.entries.iter().map(|e| e.title()).collect();
    titles.sort_unstable();
    assert_eq!(titles, ["Alpha (external)", "Beta (local)"]);
}

#[test]
fn journey_history_and_restore() {
    let w = world();
    let id = login(&w.core, "Site", "first");
    for next in ["second", "third"] {
        w.clock.advance(Duration::from_secs(5));
        let mut m = w.core.entry_edit_model(&id).unwrap();
        m.password = next.into();
        w.core.entry_save(m).unwrap();
    }
    let history = w.core.entry_history(&id).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].index, 0, "newest first");
    assert!(history[0].changed_fields.contains(&"Password".to_owned()));
    // Index 1 is the oldest version: the first password.
    w.core.entry_history_restore(&id, 1).unwrap();
    assert_eq!(
        w.core.entry_reveal(&id, FieldRef::Password).unwrap(),
        "first"
    );
    assert_eq!(w.core.entry_history(&id).unwrap().len(), 3);
}

#[test]
fn journey_recycle_bin_delete_restore_purge() {
    let w = world();
    let a = login(&w.core, "Keep", "1");
    let b = login(&w.core, "Purge", "2");
    let c = login(&w.core, "Bin one", "3");
    let d = login(&w.core, "Bin two", "4");

    w.core.entry_delete(&a).unwrap();
    assert!(w.core.entry_get(&a).unwrap().in_recycle_bin);
    assert_eq!(all(&w.core, ""), ["Bin one", "Bin two", "Purge"]);
    w.core.entry_restore(&a, None).unwrap();
    assert!(!w.core.entry_get(&a).unwrap().in_recycle_bin);

    w.core.entry_delete(&b).unwrap();
    w.core.entry_purge(&b).unwrap();
    assert_eq!(
        w.core.entry_get(&b).unwrap_err().code,
        mypass_session::ErrorCode::NotFound
    );

    w.core.entry_delete(&c).unwrap();
    w.core.entry_delete(&d).unwrap();
    assert_eq!(w.core.entry_empty_bin().unwrap(), 2);
    assert_eq!(all(&w.core, ""), ["Keep"]);
    w.clock.advance(SAVE);
    w.core.vault_lock().unwrap();
    w.core.vault_unlock(pw("pw")).unwrap();
    assert_eq!(all(&w.core, ""), ["Keep"]);
}

#[test]
fn journey_csv_import_with_quoted_fields_then_search() {
    let w = world();
    let csv = w.dir.path().join("export.csv");
    std::fs::write(
        &csv,
        "title,url,username,password,notes\n\"Acme, Inc\",https://acme.example,joe,\"pw,1\",\"multi\nline\"\nOther,https://other.example,ann,pw2,\n",
    )
    .unwrap();
    let preview = w.core.import_csv_preview(csv.to_str().unwrap()).unwrap();
    assert_eq!(preview.total, 2);
    assert_eq!(preview.sample_titles, ["Acme, Inc", "Other"]);

    let group = root(&w.core).simple().to_string();
    let result = w
        .core
        .import_csv_commit(csv.to_str().unwrap(), &group)
        .unwrap();
    assert_eq!((result.imported, result.skipped), (2, 0));
    assert_eq!(all(&w.core, "acme"), ["Acme, Inc"]);
    assert_eq!(all(&w.core, ""), ["Acme, Inc", "Other"]);
    let id = w
        .core
        .entry_list(EntryQuery {
            scope: Scope::Category {
                category: Category::All,
            },
            search: "acme".into(),
            sort: SortKey::Title,
        })
        .unwrap()[0]
        .id
        .clone();
    assert_eq!(
        w.core.entry_reveal(&id, FieldRef::Password).unwrap(),
        "pw,1"
    );
}

#[test]
fn journey_generator_history_survives_lock_unlock_and_stays_hidden() {
    let w = world();
    let generated = w
        .core
        .generator_generate(GenerateRequest {
            options: GeneratorOptionsDto::default(),
            host: None,
            record_url: Some("https://example.com/login".into()),
        })
        .unwrap();
    let history_id = generated.history_id.clone().unwrap();
    w.clock.advance(SAVE);
    w.core.vault_lock().unwrap();
    w.core.vault_unlock(pw("pw")).unwrap();

    let list = w.core.generator_history_list().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, history_id);
    assert_eq!(list[0].title, "example.com");
    assert_eq!(
        w.core.generator_history_reveal(&history_id).unwrap(),
        generated.value
    );
    // Hidden everywhere else: no entry, no sidebar group, no count.
    assert!(all(&w.core, "").is_empty());
    assert!(w.core.sidebar_get().unwrap().groups.is_empty());
}

#[test]
fn journey_backups_rotate_and_restore() {
    let w = world();
    w.core
        .settings_update(SettingsPatch {
            backup_count: Some(2),
            ..SettingsPatch::default()
        })
        .unwrap();
    for title in ["one", "two", "three", "four"] {
        login(&w.core, title, "pw");
        w.clock.advance(Duration::from_secs(2));
    }
    let status = w.core.backup_list().unwrap();
    assert_eq!(status.items.len(), 2, "rotated to the configured count");
    // Newest first: [one, two, three] then the older [one, two].
    let older = status.items[1].id.clone();
    w.core.backup_restore(&older).unwrap();
    assert_eq!(all(&w.core, ""), ["one", "two"]);
    let on_disk = VaultFile::open(&w.path, &pw("pw")).unwrap().1;
    assert_eq!(on_disk.root.entries.len(), 2);
}

#[test]
fn journey_vault_opens_in_stage1_oracle() {
    let w = world();
    login(&w.core, "Oracle entry", "oracle-pw");
    w.clock.advance(SAVE);
    w.core.vault_lock().unwrap();
    let bytes = PlainFileIo.read(&w.path).unwrap();
    let oracle = keepass::Database::open(
        &mut bytes.as_slice(),
        DatabaseKey::new().with_password("pw"),
    )
    .unwrap();
    let root = oracle.root();
    let entry = root
        .entries()
        .find(|e| e.get_title() == Some("Oracle entry"))
        .expect("the entry is in the oracle's view");
    assert_eq!(entry.get_password(), Some("oracle-pw"));
}
