#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use super::*;
use std::fs;

const KDBX: [u8; 12] = [
    0x03, 0xD9, 0xA2, 0x9A, 0x67, 0xFB, 0x4B, 0xB5, 0x01, 0x00, 0x04, 0x00,
];

fn sha(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// A folder with a live vault `v.kdbx` and one competing copy next to it.
struct Fx {
    _dir: tempfile::TempDir,
    root: PathBuf,
    vault: PathBuf,
    copy: PathBuf,
}

fn fx() -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    let vault = root.join("v.kdbx");
    fs::write(&vault, [KDBX.as_slice(), b"live"].concat()).unwrap();
    let copy = root.join(".v.kdbx.1.2.3.displaced");
    fs::write(&copy, b"competing").unwrap();
    Fx {
        _dir: dir,
        root,
        vault,
        copy,
    }
}

fn removal(target: &Path, copy: &Path, bytes: &[u8]) -> RetainedRemoval {
    RetainedRemoval {
        target: target.to_path_buf(),
        name: copy.file_name().unwrap().to_os_string(),
        len: bytes.len() as u64,
        sha256: sha(bytes),
    }
}

fn refusal(result: Result<()>) -> RetainedRefusal {
    match result {
        Err(KdbxError::RetainedCopyRefused(reason)) => reason,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut all: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    all.sort();
    all
}

fn no_seams() -> (impl FnMut(), impl FnMut(&Path) -> io::Result<()>) {
    (|| {}, remove_private_name)
}

#[test]
fn a_verified_copy_is_deleted_and_nothing_else_is_touched() {
    let f = fx();
    remove_retained_copy(&removal(&f.vault, &f.copy, b"competing"), &[]).unwrap();
    assert!(!f.copy.exists());
    assert_eq!(names(&f.root), ["v.kdbx"]);
    assert_eq!(&fs::read(&f.vault).unwrap()[..12], KDBX.as_slice());
}

#[test]
fn the_live_vault_itself_is_never_deleted() {
    let f = fx();
    let live = fs::read(&f.vault).unwrap();
    let r = removal(&f.vault, &f.vault, &live);
    assert_eq!(
        refusal(remove_retained_copy(&r, &[])),
        RetainedRefusal::NotRetainedCopy
    );
    // A path that walks to the vault is not a bare copy name either.
    let mut walk = r.clone();
    walk.name = OsString::from("../v.kdbx");
    assert_eq!(
        refusal(remove_retained_copy(&walk, &[])),
        RetainedRefusal::NotRetainedCopy
    );
    assert_eq!(fs::read(&f.vault).unwrap(), live);
}

#[test]
fn a_copy_is_kept_while_the_live_vault_is_missing_or_not_a_vault() {
    let f = fx();
    fs::remove_file(&f.vault).unwrap();
    let r = removal(&f.vault, &f.copy, b"competing");
    assert_eq!(
        refusal(remove_retained_copy(&r, &[])),
        RetainedRefusal::LiveVaultMissing
    );
    fs::write(&f.vault, b"not a kdbx file at all").unwrap();
    assert_eq!(
        refusal(remove_retained_copy(&r, &[])),
        RetainedRefusal::LiveVaultMissing
    );
    assert_eq!(fs::read(&f.copy).unwrap(), b"competing");
}

#[test]
fn a_copy_swapped_after_listing_is_refused() {
    let f = fx();
    let r = removal(&f.vault, &f.copy, b"competing");
    // Same name, same length, different bytes.
    fs::write(&f.copy, b"COMPETING").unwrap();
    assert_eq!(
        refusal(remove_retained_copy(&r, &[])),
        RetainedRefusal::Changed
    );
    fs::write(&f.copy, b"longer competing").unwrap();
    assert_eq!(
        refusal(remove_retained_copy(&r, &[])),
        RetainedRefusal::Changed
    );
    assert_eq!(fs::read(&f.copy).unwrap(), b"longer competing");
}

#[test]
fn a_copy_swapped_between_the_checks_and_the_move_is_put_back() {
    let f = fx();
    let r = removal(&f.vault, &f.copy, b"competing");
    let copy = f.copy.clone();
    let mut swap = move || {
        let other = copy.with_file_name("other");
        fs::write(&other, b"another writer").unwrap();
        fs::rename(&other, &copy).unwrap();
    };
    let mut unlink = remove_private_name;
    let result = remove_with(
        &r,
        &[],
        SystemTime::now(),
        &mut RemoveSeams {
            before_move: &mut swap,
            unlink: &mut unlink,
        },
    );
    assert_eq!(refusal(result), RetainedRefusal::Changed);
    assert_eq!(fs::read(&f.copy).unwrap(), b"another writer");
    assert_eq!(names(&f.root), [".v.kdbx.1.2.3.displaced", "v.kdbx"]);
}

#[test]
fn a_failed_unlink_leaves_the_copy_listed() {
    let f = fx();
    let r = removal(&f.vault, &f.copy, b"competing");
    let mut before = || {};
    let mut unlink = |_: &Path| Err(io::Error::other("injected unlink failure"));
    let result = remove_with(
        &r,
        &[],
        SystemTime::now(),
        &mut RemoveSeams {
            before_move: &mut before,
            unlink: &mut unlink,
        },
    );
    assert!(matches!(result, Err(KdbxError::Io { .. })), "{result:?}");
    let listed = displaced_copies(&f.vault).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(fs::read(&listed[0].path).unwrap(), b"competing");
}

#[cfg(unix)]
#[test]
fn a_symlink_at_the_listed_name_is_refused() {
    let f = fx();
    let r = removal(&f.vault, &f.copy, b"competing");
    let real = f.root.join("elsewhere");
    fs::rename(&f.copy, &real).unwrap();
    std::os::unix::fs::symlink(&real, &f.copy).unwrap();
    assert_eq!(
        refusal(remove_retained_copy(&r, &[])),
        RetainedRefusal::NotRegularFile
    );
    // A link to the live vault is refused the same way.
    fs::remove_file(&f.copy).unwrap();
    std::os::unix::fs::symlink(&f.vault, &f.copy).unwrap();
    let live = fs::read(&f.vault).unwrap();
    assert_eq!(
        refusal(remove_retained_copy(
            &removal(&f.vault, &f.copy, &live),
            &[]
        )),
        RetainedRefusal::NotRegularFile
    );
    assert!(real.exists() && f.vault.exists());
    assert!(fs::symlink_metadata(&f.copy)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[cfg(unix)]
#[test]
fn a_hard_link_of_the_live_vault_is_refused() {
    let f = fx();
    fs::remove_file(&f.copy).unwrap();
    fs::hard_link(&f.vault, &f.copy).unwrap();
    let live = fs::read(&f.vault).unwrap();
    assert_eq!(
        refusal(remove_retained_copy(
            &removal(&f.vault, &f.copy, &live),
            &[]
        )),
        RetainedRefusal::NotRegularFile
    );
    assert!(f.copy.exists());
}

#[test]
fn a_copy_recorded_for_another_vault_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    // Two long names with the same first 96 bytes share retained names.
    let stem = "x".repeat(100);
    let a = root.join(format!("{stem}-a.kdbx"));
    let b = root.join(format!("{stem}-b.kdbx"));
    fs::write(&a, [KDBX.as_slice(), b"a"].concat()).unwrap();
    fs::write(&b, [KDBX.as_slice(), b"b"].concat()).unwrap();
    let copy = root.join(format!(".{}.1.2.3.displaced", &stem[..96]));
    fs::write(&copy, b"kept for b").unwrap();
    let record = RetainedIdentity {
        target: b.clone(),
        path: copy.clone(),
        len: 10,
        sha256: sha(b"kept for b"),
    };
    let for_a = removal(&a, &copy, b"kept for b");
    assert_eq!(
        refusal(remove_retained_copy(&for_a, std::slice::from_ref(&record))),
        RetainedRefusal::OtherVault
    );
    // Without a record a truncated name cannot tell the vaults apart.
    assert_eq!(
        refusal(remove_retained_copy(&for_a, &[])),
        RetainedRefusal::UnverifiedVault
    );
    let inspected = inspect_retained_copies(&a, std::slice::from_ref(&record)).unwrap();
    assert_eq!(
        inspected.copies[0].verdict,
        Err(RetainedRefusal::OtherVault)
    );
    assert!(copy.exists());
    // The vault it was recorded for may delete it.
    remove_retained_copy(&removal(&b, &copy, b"kept for b"), &[record]).unwrap();
    assert!(!copy.exists());
}

#[test]
fn a_record_that_no_longer_matches_the_bytes_is_refused() {
    let f = fx();
    let record = RetainedIdentity {
        target: f.vault.clone(),
        path: f.copy.clone(),
        len: 9,
        sha256: sha(b"something"),
    };
    let r = removal(&f.vault, &f.copy, b"competing");
    assert_eq!(
        refusal(remove_retained_copy(&r, std::slice::from_ref(&record))),
        RetainedRefusal::Changed
    );
    let checked = inspect_retained_copies(&f.vault, &[record]).unwrap();
    assert_eq!(checked.copies[0].verdict, Err(RetainedRefusal::Changed));
    assert!(f.copy.exists());
}

#[test]
fn only_stems_that_cannot_be_shared_allow_unrecorded_removal() {
    for (len, suffix, expected) in [
        (92, "\u{1f4a1}", RetainedRefusal::NotRetainedCopy),
        (93, "\u{1f4a1}", RetainedRefusal::UnverifiedVault),
        (96, "x", RetainedRefusal::UnverifiedVault),
        (97, "x", RetainedRefusal::UnverifiedVault),
    ] {
        let f = fx();
        let name = "a".repeat(len);
        let longer = format!("{name}{suffix}");
        let a = f.root.join(&name);
        let b = f.root.join(&longer);
        fs::copy(&f.vault, &a).unwrap();
        fs::copy(&f.vault, b).unwrap();
        let own_copy = f.root.join(format!(
            ".{}.1.2.3.displaced",
            super::super::sibling_stem(&name)
        ));
        let other_copy = f.root.join(format!(
            ".{}.1.2.4.displaced",
            super::super::sibling_stem(&longer)
        ));
        fs::write(&own_copy, b"kept for a").unwrap();
        fs::write(&other_copy, b"kept for b").unwrap();
        let checked = inspect_retained_copies(&a, &[]).unwrap();
        if len == 92 {
            assert_eq!(checked.copies.len(), 1);
            assert_eq!(
                checked.copies[0].verdict,
                Ok(RetainedProvenance::Unrecorded)
            );
        } else {
            assert_eq!(checked.copies.len(), 2, "{len}-byte vault name");
            assert!(
                checked.copies.iter().all(|c| c.verdict == Err(expected)),
                "{len}-byte vault name: {checked:?}"
            );
        }
        assert_eq!(
            refusal(remove_retained_copy(
                &removal(&a, &other_copy, b"kept for b"),
                &[]
            )),
            expected,
            "{len}-byte vault name"
        );
        assert_eq!(fs::read(other_copy).unwrap(), b"kept for b");
    }
}

#[cfg(unix)]
#[test]
fn renaming_old_contents_to_a_working_name_starts_the_grace() {
    let f = fx();
    let old_mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    fs::File::options()
        .write(true)
        .open(&f.copy)
        .unwrap()
        .set_modified(old_mtime)
        .unwrap();
    let aside = f.root.join(".v.kdbx.1.2.4.aside");
    fs::rename(&f.copy, &aside).unwrap();
    assert_eq!(fs::metadata(&aside).unwrap().modified().unwrap(), old_mtime);

    let checked = inspect_retained_copies(&f.vault, &[]).unwrap();
    assert_eq!(checked.copies[0].verdict, Err(RetainedRefusal::InFlight));
    assert_eq!(
        refusal(remove_retained_copy(
            &removal(&f.vault, &aside, b"competing"),
            &[]
        )),
        RetainedRefusal::InFlight
    );
    assert_eq!(fs::read(aside).unwrap(), b"competing");
}

#[test]
fn inspection_hashes_and_judges_every_listed_copy() {
    let f = fx();
    let aside = f.root.join(".v.kdbx.1.2.4.aside");
    fs::write(&aside, b"rollback").unwrap();
    fs::write(f.root.join(".v.kdbx.1.2.5.tmp"), b"candidate").unwrap();
    let checked = inspect_retained_copies(&f.vault, &[]).unwrap();
    assert_eq!(checked.target, f.vault);
    assert_eq!(checked.copies.len(), 2, "temp names are never listed");
    let copy = checked.copies.iter().find(|c| c.path == f.copy).unwrap();
    assert_eq!(copy.sha256, Some(sha(b"competing")));
    assert_eq!(copy.verdict, Ok(RetainedProvenance::Unrecorded));
    // A fresh working file may belong to a replacement still running elsewhere.
    let fresh = checked.copies.iter().find(|c| c.path == aside).unwrap();
    assert_eq!(fresh.verdict, Err(RetainedRefusal::InFlight));
    let r = removal(&f.vault, &aside, b"rollback");
    assert_eq!(
        refusal(remove_retained_copy(&r, &[])),
        RetainedRefusal::InFlight
    );
    // Once settled it may go.
    let (mut before, mut unlink) = no_seams();
    remove_with(
        &r,
        &[],
        SystemTime::now() + RETAINED_IN_FLIGHT_GRACE + Duration::from_secs(1),
        &mut RemoveSeams {
            before_move: &mut before,
            unlink: &mut unlink,
        },
    )
    .unwrap();
    assert!(!aside.exists());
}

#[test]
fn temp_and_in_process_working_names_are_never_deletable() {
    let f = fx();
    let temp = f.root.join(".v.kdbx.1.2.6.tmp");
    fs::write(&temp, b"candidate").unwrap();
    assert_eq!(
        refusal(remove_retained_copy(
            &removal(&f.vault, &temp, b"candidate"),
            &[]
        )),
        RetainedRefusal::NotRetainedCopy
    );
    let working = f.root.join(".v.kdbx.1.2.7.aside");
    fs::write(&working, b"rollback").unwrap();
    let _active = ActiveAside::begin(&working);
    let (mut before, mut unlink) = no_seams();
    let result = remove_with(
        &removal(&f.vault, &working, b"rollback"),
        &[],
        SystemTime::now() + RETAINED_IN_FLIGHT_GRACE * 2,
        &mut RemoveSeams {
            before_move: &mut before,
            unlink: &mut unlink,
        },
    );
    assert_eq!(refusal(result), RetainedRefusal::InFlight);
    assert!(temp.exists() && working.exists());
}

#[test]
fn a_displaced_outcome_records_the_exact_identity_of_the_kept_copy() {
    let f = fx();
    let outcome = Ok(ReplaceOutcome::Displaced {
        bytes: b"competing".to_vec(),
        kept: f.copy.clone(),
        leftover: None,
    });
    record_outcome(&f.vault, &outcome);
    let recorded = recorded_retained_copies();
    let mine = recorded.iter().find(|r| r.path == f.copy).unwrap();
    assert_eq!(mine.target, f.vault);
    assert_eq!(mine.len, 9);
    assert_eq!(mine.sha256, sha(b"competing"));
    // A temp name is never recorded.
    let temp = f.root.join(".v.kdbx.1.2.8.tmp");
    fs::write(&temp, b"candidate").unwrap();
    record_outcome(
        &f.vault,
        &Ok(ReplaceOutcome::Displaced {
            bytes: Vec::new(),
            kept: temp.clone(),
            leftover: None,
        }),
    );
    assert!(!recorded_retained_copies().iter().any(|r| r.path == temp));
}

#[test]
fn a_real_replacement_race_records_its_retained_copy() {
    // The stranded failure path keeps the previous vault under a retained working name and
    // reports it in the error: that file is recorded at once.
    let f = fx();
    let live = fs::read(&f.vault).unwrap();
    let result =
        super::super::replace_if_injected(&f.vault, &sha(&live), b"new", 1 << 20, true, None);
    let kept = match &result {
        Err(KdbxError::VaultRetained { path, .. }) => path.clone(),
        Ok(outcome) => match outcome.retained_working_path() {
            Some(path) => path.to_path_buf(),
            None => return,
        },
        Err(other) => panic!("{other:?}"),
    };
    let listed = displaced_kind("v.kdbx", &kept.file_name().unwrap().to_string_lossy());
    if listed.is_some() {
        let recorded = recorded_retained_copies();
        let mine = recorded.iter().find(|r| r.path == kept).expect("recorded");
        assert_eq!(mine.sha256, sha(&fs::read(&kept).unwrap()));
        assert_eq!(mine.target, f.vault);
    }
}
