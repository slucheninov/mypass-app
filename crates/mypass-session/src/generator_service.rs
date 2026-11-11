//! The password generator service and the generator history.
//!
//! Generation is stage 1 code in `mypass-core`; this module validates the request, looks up the
//! site rules, calls the generator and, when asked, records the value in the hidden history group.

use crate::dto::{
    timestamp_ms, GenerateRequest, GeneratedPassword, GeneratorHistoryItem, RulesSummary,
};
use crate::fields::GENERATOR_HISTORY_MARKER;
use crate::session::{GeneratorRulesError, SessionError, VaultSession};
use crate::settings::{GeneratorOptionsDto, GENERATOR_LENGTH};
use mypass_core::entry_ext::GENERATOR_HISTORY_GROUP_UUID;
use mypass_core::generator_toggles::{
    excluded_toggle_classes, forced_on_toggle_classes, TOGGLE_BACKED_CLASSES,
};
use mypass_core::matcher::host_of;
use mypass_core::psl::canonical_host;
use mypass_core::{
    bundled_rules_for_host, generate_os, CharacterClass, GeneratorError, GeneratorOptions,
    PasswordRules,
};
use mypass_kdbx::{Database, Entry, Group, Timestamp};
use std::collections::BTreeSet;
use uuid::Uuid;

/// The history keeps this many generated passwords.
pub const HISTORY_LIMIT: usize = 30;
/// Name of the hidden history group.
pub const HISTORY_GROUP_NAME: &str = "Password Generator History";

const NO_CLASS: SessionError = SessionError::GeneratorRules(GeneratorRulesError::NoClass);
const UNSATISFIABLE: SessionError =
    SessionError::GeneratorRules(GeneratorRulesError::Unsatisfiable);

fn class_name(class: &CharacterClass) -> Option<&'static str> {
    match class {
        CharacterClass::Lower => Some("lowercase"),
        CharacterClass::Upper => Some("uppercase"),
        CharacterClass::Digit => Some("digits"),
        CharacterClass::Special => Some("symbols"),
        _ => None,
    }
}

/// Class names in the fixed display order (lowercase, uppercase, digits, symbols).
fn class_names(set: &BTreeSet<CharacterClass>) -> Vec<String> {
    TOGGLE_BACKED_CLASSES
        .iter()
        .filter(|c| set.contains(c))
        .filter_map(class_name)
        .map(str::to_owned)
        .collect()
}

fn summarize(rules: &PasswordRules) -> RulesSummary {
    RulesSummary {
        min_length: rules.min_length,
        max_length: rules.max_length,
        max_consecutive: rules.max_consecutive,
        forced_on: class_names(&forced_on_toggle_classes(rules)),
        excluded: class_names(&excluded_toggle_classes(rules)),
    }
}

fn rules_for(host: &str) -> Option<PasswordRules> {
    bundled_rules_for_host(&canonical_host(host)?)
}

/// The bundled site rules for `host`, as the generator panel shows them.
pub fn rules_summary(host: &str) -> Option<RulesSummary> {
    rules_for(host).as_ref().map(summarize)
}

/// Pure: the options a generation actually uses. Length is clamped to the site's bounds, forced
/// classes are switched on and excluded ones off. `length` outside 8..=64 is `InvalidInput`; no
/// class (and none forced) is `NoClass`; every selected class excluded is `Unsatisfiable`.
pub fn resolve_options(
    options: &GeneratorOptionsDto,
    rules: Option<&RulesSummary>,
) -> Result<GeneratorOptionsDto, SessionError> {
    if !GENERATOR_LENGTH.contains(&options.length) {
        return Err(SessionError::InvalidInput(
            "generator length out of range".to_owned(),
        ));
    }
    let selected = |name: &str, on: bool, rules: Option<&RulesSummary>| {
        let in_list = |list: &[String]| list.iter().any(|c| c == name);
        match rules {
            Some(r) => in_list(&r.forced_on) || (on && !in_list(&r.excluded)),
            None => on,
        }
    };
    let none_requested =
        !(options.lowercase || options.uppercase || options.digits || options.symbols)
            && rules.is_none_or(|r| r.forced_on.is_empty());
    if none_requested {
        return Err(NO_CLASS);
    }
    let mut length = options.length;
    if let Some(r) = rules {
        if let Some(min) = r.min_length {
            length = length.max(min);
        }
        if let Some(max) = r.max_length {
            length = length.min(max);
        }
    }
    let effective = GeneratorOptionsDto {
        length,
        lowercase: selected("lowercase", options.lowercase, rules),
        uppercase: selected("uppercase", options.uppercase, rules),
        digits: selected("digits", options.digits, rules),
        symbols: selected("symbols", options.symbols, rules),
    };
    if !(effective.lowercase || effective.uppercase || effective.digits || effective.symbols) {
        return Err(UNSATISFIABLE);
    }
    Ok(effective)
}

fn map_generator_error(e: GeneratorError) -> SessionError {
    match e {
        GeneratorError::NoCharacterClassSelected => NO_CLASS,
        GeneratorError::UnsatisfiableRules => UNSATISFIABLE,
    }
}

/// Generates a password. The session is needed only when `req.record_url` is `Some`: without it
/// the vault is never touched, so this works with no vault and with a locked one. With a record
/// URL and no unlocked session the session's error is returned and no value is.
pub fn generate_password(
    session: Option<&VaultSession>,
    req: &GenerateRequest,
) -> Result<GeneratedPassword, SessionError> {
    let site = req.host.as_deref().and_then(rules_for);
    let summary = site.as_ref().map(summarize);
    let effective = resolve_options(&req.options, summary.as_ref())?;
    let core_options = GeneratorOptions {
        length: effective.length as usize,
        lowercase: effective.lowercase,
        uppercase: effective.uppercase,
        digits: effective.digits,
        symbols: effective.symbols,
    };
    let value = generate_os(&core_options, site.as_ref()).map_err(map_generator_error)?;
    let history_id = match &req.record_url {
        Some(url) => {
            let session = session.ok_or(SessionError::NoVault)?;
            Some(record_history(session, &value, url)?.simple().to_string())
        }
        None => None,
    };
    Ok(GeneratedPassword {
        value: String::clone(&value),
        effective,
        rules: summary,
        history_id,
    })
}

/// Entries of the history group in insertion order, with their index.
fn history_entries(db: &Database) -> Vec<&Entry> {
    db.find_group(GENERATOR_HISTORY_GROUP_UUID)
        .map(|g| g.entries.iter().collect())
        .unwrap_or_default()
}

/// Newest first; entries created in the same second list the later insert first.
pub fn history_list(db: &Database) -> Vec<GeneratorHistoryItem> {
    let mut entries = history_entries(db);
    entries.reverse();
    entries.sort_by_key(|e| std::cmp::Reverse(e.created));
    entries
        .into_iter()
        .map(|e| GeneratorHistoryItem {
            id: e.uuid.simple().to_string(),
            url: e.field("URL").unwrap_or_default().to_owned(),
            title: e.field("Title").unwrap_or_default().to_owned(),
            created: timestamp_ms(e.created),
        })
        .collect()
}

/// Writes `value` to the history group (created on first use) and purges entries beyond
/// [`HISTORY_LIMIT`], oldest first, leaving tombstones. One mutation: all or nothing.
pub fn record_history(
    session: &VaultSession,
    value: &str,
    url: &str,
) -> Result<Uuid, SessionError> {
    session.mutate(|db, now| {
        if db.find_group(GENERATOR_HISTORY_GROUP_UUID).is_none() {
            let mut group = Group::new(HISTORY_GROUP_NAME, now);
            group.uuid = GENERATOR_HISTORY_GROUP_UUID;
            group.set_custom(GENERATOR_HISTORY_MARKER, "1");
            let root = db.root_id();
            db.add_group(root, group, now)?;
        }
        let mut entry = Entry::new(now);
        let title = host_of(url).unwrap_or_else(|| url.to_owned());
        entry.set_field("Title", &title, false);
        entry.set_field("URL", url, false);
        entry.set_field("Password", value, true);
        let id = entry.uuid;
        db.add_entry(GENERATOR_HISTORY_GROUP_UUID, entry, now)?;

        let mut ordered: Vec<(Timestamp, Uuid)> = history_entries(db)
            .into_iter()
            .map(|e| (e.created, e.uuid))
            .collect();
        // Stable sort: equal timestamps keep insertion order, so the earlier insert is older.
        ordered.sort_by_key(|(created, _)| *created);
        let excess = ordered.len().saturating_sub(HISTORY_LIMIT);
        for (_, old) in ordered.into_iter().take(excess) {
            db.purge_entry(old, now)?;
        }
        Ok(id)
    })
}

/// The recorded password of history entry `id`; `NotFound` for anything outside the history.
pub fn history_reveal(db: &Database, id: Uuid) -> Result<String, SessionError> {
    history_entries(db)
        .into_iter()
        .find(|e| e.uuid == id)
        .map(|e| e.field("Password").unwrap_or_default().to_owned())
        .ok_or(SessionError::NotFound)
}

/// Permanently removes history entry `id` (with a tombstone); `NotFound` for anything else.
pub fn history_delete(session: &VaultSession, id: Uuid) -> Result<(), SessionError> {
    session.mutate(|db, now| {
        if db.parent_of_entry(id) != Some(GENERATOR_HISTORY_GROUP_UUID) {
            return Err(SessionError::NotFound);
        }
        db.purge_entry(id, now)?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{Category, EntryQuery, Scope, SortKey};
    use crate::fields::GENERATOR_HISTORY_MARKER;
    use crate::queries::{list_entries, sidebar};
    use crate::scheduler::ManualScheduler;
    use crate::session::{SaveState, SessionDeps};
    use crate::settings::SettingsStore;
    use crate::testing::fast_database;
    use mypass_core::entry_ext::GENERATOR_HISTORY_GROUP_UUID;
    use mypass_core::matcher::credential_candidates;
    use mypass_kdbx::{Entry, PlainFileIo, Timestamp};
    use secrecy::SecretString;
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};
    use tempfile::TempDir;

    struct Fx {
        _dir: TempDir,
        session: VaultSession,
        clock: Arc<ManualScheduler>,
    }

    fn fx() -> Fx {
        let dir = tempfile::tempdir().unwrap();
        let clock = ManualScheduler::new(UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        let session = VaultSession::new(SessionDeps {
            settings: SettingsStore::in_memory(),
            scheduler: clock.clone(),
            backups_root: dir.path().join("backups"),
            file_io: Arc::new(PlainFileIo),
        });
        session
            .create_vault_with(
                &dir.path().join("v.kdbx"),
                SecretString::from("pw".to_owned()),
                fast_database("T"),
            )
            .unwrap();
        Fx {
            _dir: dir,
            session,
            clock,
        }
    }

    fn opts() -> GeneratorOptionsDto {
        GeneratorOptionsDto::default()
    }

    fn req(options: GeneratorOptionsDto) -> GenerateRequest {
        GenerateRequest {
            options,
            host: None,
            record_url: None,
        }
    }

    fn gen(options: GeneratorOptionsDto) -> Result<GeneratedPassword, SessionError> {
        generate_password(None, &req(options))
    }

    fn record(f: &Fx, value: &str, url: &str) -> Uuid {
        record_history(&f.session, value, url).unwrap()
    }

    #[test]
    fn default_options_generate_twenty_chars_all_classes() {
        let (mut lower, mut upper, mut digit, mut symbol) = (false, false, false, false);
        for _ in 0..50 {
            let g = gen(opts()).unwrap();
            assert_eq!(g.value.chars().count(), 20);
            assert_eq!(g.effective, opts());
            assert!(g.rules.is_none() && g.history_id.is_none());
            lower |= g.value.chars().any(|c| c.is_ascii_lowercase());
            upper |= g.value.chars().any(|c| c.is_ascii_uppercase());
            digit |= g.value.chars().any(|c| c.is_ascii_digit());
            symbol |= g.value.chars().any(|c| !c.is_ascii_alphanumeric());
        }
        assert!(lower && upper && digit && symbol);
    }

    #[test]
    fn length_7_and_65_rejected_8_and_64_accepted() {
        for bad in [7, 65] {
            let r = gen(GeneratorOptionsDto {
                length: bad,
                ..opts()
            });
            assert!(matches!(r, Err(SessionError::InvalidInput(_))), "{bad}");
        }
        for good in [8, 64] {
            let g = gen(GeneratorOptionsDto {
                length: good,
                ..opts()
            })
            .unwrap();
            assert_eq!(g.value.chars().count(), good as usize);
        }
    }

    #[test]
    fn no_class_selected_is_generator_rules_no_class() {
        let none = GeneratorOptionsDto {
            lowercase: false,
            uppercase: false,
            digits: false,
            symbols: false,
            ..opts()
        };
        assert_eq!(
            gen(none.clone()).unwrap_err(),
            SessionError::GeneratorRules(GeneratorRulesError::NoClass)
        );
        assert_eq!(
            resolve_options(&none, None).unwrap_err(),
            SessionError::GeneratorRules(GeneratorRulesError::NoClass)
        );
    }

    #[test]
    fn site_rules_excluding_every_selected_class_is_unsatisfiable() {
        let rules = RulesSummary {
            min_length: None,
            max_length: None,
            max_consecutive: None,
            forced_on: vec![],
            excluded: vec!["lowercase".into(), "uppercase".into()],
        };
        let only_letters = GeneratorOptionsDto {
            digits: false,
            symbols: false,
            ..opts()
        };
        assert_eq!(
            resolve_options(&only_letters, Some(&rules)).unwrap_err(),
            SessionError::GeneratorRules(GeneratorRulesError::Unsatisfiable)
        );
        let ok = resolve_options(&opts(), Some(&rules)).unwrap();
        assert!(!ok.lowercase && !ok.uppercase && ok.digits && ok.symbols);
    }

    #[test]
    fn site_rules_clamp_length_and_force_classes() {
        let g = generate_password(
            None,
            &GenerateRequest {
                options: opts(),
                host: Some("account.samsung.com".into()),
                record_url: None,
            },
        )
        .unwrap();
        let rules = g.rules.unwrap();
        assert_eq!(g.value.chars().count(), 15);
        assert_eq!(g.effective.length, 15);
        assert_eq!(rules.max_length, Some(15));
        // The core forces every class named by a `required` clause, so the `upper,lower` clause
        // forces both letter classes on besides digits and symbols.
        assert_eq!(
            rules.forced_on,
            ["lowercase", "uppercase", "digits", "symbols"]
        );
        let letters_off = GeneratorOptionsDto {
            lowercase: false,
            uppercase: false,
            ..opts()
        };
        let g = generate_password(
            None,
            &GenerateRequest {
                options: letters_off,
                host: Some("account.samsung.com".into()),
                record_url: None,
            },
        )
        .unwrap();
        assert!(g.effective.lowercase && g.effective.uppercase);
    }

    #[test]
    fn resolve_options_agrees_with_core_effective_options() {
        use mypass_core::generator_toggles::effective_options;
        use mypass_core::password_rules::parse_password_rules;
        for raw in [
            "minlength: 6; maxlength: 6; allowed: digit; max-consecutive: 3",
            "minlength: 8; maxlength: 40; required: upper; required: [!#$%&*@^]; allowed: lower,digit;",
            "minlength: 10; required: lower; required: upper; required: digit; required: special;",
            "minlength: 8; maxlength: 15; required: digit; required: upper,lower;",
        ] {
            let rules = parse_password_rules(raw).unwrap();
            let summary = summarize(&rules);
            for mask in 0..16u8 {
                let dto = GeneratorOptionsDto {
                    length: 20,
                    lowercase: mask & 1 != 0,
                    uppercase: mask & 2 != 0,
                    digits: mask & 4 != 0,
                    symbols: mask & 8 != 0,
                };
                let base = GeneratorOptions {
                    length: 20,
                    lowercase: dto.lowercase,
                    uppercase: dto.uppercase,
                    digits: dto.digits,
                    symbols: dto.symbols,
                };
                let core = effective_options(&base, &rules);
                let any = core.lowercase || core.uppercase || core.digits || core.symbols;
                match resolve_options(&dto, Some(&summary)) {
                    Ok(e) => {
                        assert!(any, "{raw} {mask}");
                        assert_eq!(
                            (e.length as usize, e.lowercase, e.uppercase, e.digits, e.symbols),
                            (core.length, core.lowercase, core.uppercase, core.digits, core.symbols),
                            "{raw} {mask}"
                        );
                    }
                    Err(_) => assert!(!any, "{raw} {mask}"),
                }
            }
        }
    }

    #[test]
    fn unknown_host_has_no_rules() {
        assert_eq!(rules_summary("no-such-site.invalid"), None);
        let g = generate_password(
            None,
            &GenerateRequest {
                options: opts(),
                host: Some("no-such-site.invalid".into()),
                record_url: None,
            },
        )
        .unwrap();
        assert!(g.rules.is_none());
    }

    #[test]
    fn rules_summary_for_known_host_matches_generation() {
        let summary = rules_summary("account.samsung.com").unwrap();
        let g = generate_password(
            None,
            &GenerateRequest {
                options: opts(),
                host: Some("Account.Samsung.com".into()),
                record_url: None,
            },
        )
        .unwrap();
        assert_eq!(g.rules, Some(summary));
    }

    #[test]
    fn recording_creates_group_with_fixed_uuid_name_and_marker() {
        assert_eq!(
            GENERATOR_HISTORY_GROUP_UUID.to_string().to_uppercase(),
            "E4B1C0A2-7D58-4F13-9A6E-2C8F5B0D11A7"
        );
        let f = fx();
        f.session
            .with_db(|db| assert!(db.find_group(GENERATOR_HISTORY_GROUP_UUID).is_none()))
            .unwrap();
        let g = generate_password(
            Some(&f.session),
            &GenerateRequest {
                record_url: Some("https://example.com/login".into()),
                ..req(opts())
            },
        )
        .unwrap();
        assert!(g.history_id.is_some());
        f.session
            .with_db(|db| {
                let group = db.find_group(GENERATOR_HISTORY_GROUP_UUID).unwrap();
                assert_eq!(group.name, HISTORY_GROUP_NAME);
                assert_eq!(group.custom(GENERATOR_HISTORY_MARKER), Some("1"));
                assert_eq!(group.entries.len(), 1);
            })
            .unwrap();
    }

    #[test]
    fn recording_stores_url_host_title_and_protected_password() {
        let f = fx();
        let id = record(&f, "s3cret-value", "https://login.example.com/a?b=1");
        let now = Timestamp(1_700_000_000);
        f.session
            .with_db(|db| {
                let e = db.find_entry(id).unwrap();
                assert_eq!(e.field("Title"), Some("login.example.com"));
                assert_eq!(e.field("URL"), Some("https://login.example.com/a?b=1"));
                assert_eq!(e.field("Password"), Some("s3cret-value"));
                let pw = e.fields.iter().find(|x| x.key == "Password").unwrap();
                assert!(pw.protected);
                assert_eq!((e.created, e.modified, e.accessed), (now, now, now));
                assert_eq!(db.parent_of_entry(id), Some(GENERATOR_HISTORY_GROUP_UUID));
            })
            .unwrap();
    }

    #[test]
    fn history_list_newest_first_without_values() {
        let f = fx();
        let a = record(&f, "first-password-AAA", "https://a.example.com");
        f.clock.advance(Duration::from_secs(10));
        let b = record(&f, "second-password-BBB", "https://b.example.com");
        let items = f.session.with_db(history_list).unwrap();
        assert_eq!(
            items.iter().map(|i| i.id.clone()).collect::<Vec<_>>(),
            [b.simple().to_string(), a.simple().to_string()]
        );
        assert_eq!(items[0].title, "b.example.com");
        assert_eq!(items[0].url, "https://b.example.com");
        assert_eq!(items[0].created, 1_700_000_010_000.0);
        let json = serde_json::to_string(&items).unwrap();
        assert!(!json.contains("password-AAA") && !json.contains("password-BBB"));
    }

    #[test]
    fn history_cap_30_purges_oldest_with_tombstone() {
        let f = fx();
        let mut ids = Vec::new();
        for i in 0..31 {
            ids.push(record(&f, &format!("pw-{i}"), "https://example.com"));
            f.clock.advance(Duration::from_secs(1));
        }
        f.session
            .with_db(|db| {
                assert_eq!(history_list(db).len(), HISTORY_LIMIT);
                assert!(db.find_entry(ids[0]).is_none());
                assert!(db.find_entry(ids[30]).is_some());
                assert_eq!(db.deleted_objects.len(), 1);
                assert_eq!(db.deleted_objects[0].uuid, ids[0]);
            })
            .unwrap();
    }

    #[test]
    fn history_reveal_returns_value_and_delete_removes() {
        let f = fx();
        let id = record(&f, "reveal-me", "https://example.com");
        assert_eq!(
            f.session.with_db(|db| history_reveal(db, id)).unwrap(),
            Ok("reveal-me".to_owned())
        );
        // An entry outside the history group is not revealable or deletable through it.
        let other = f
            .session
            .mutate(|db, now| {
                let mut e = Entry::new(now);
                e.set_field("Password", "private", true);
                let id = e.uuid;
                let root = db.root_id();
                db.add_entry(root, e, now)?;
                Ok(id)
            })
            .unwrap();
        assert_eq!(
            f.session.with_db(|db| history_reveal(db, other)).unwrap(),
            Err(SessionError::NotFound)
        );
        assert_eq!(
            history_delete(&f.session, other),
            Err(SessionError::NotFound)
        );
        history_delete(&f.session, id).unwrap();
        f.session
            .with_db(|db| {
                assert!(history_list(db).is_empty());
                assert_eq!(history_reveal(db, id), Err(SessionError::NotFound));
                assert_eq!(db.deleted_objects.len(), 1);
            })
            .unwrap();
        assert_eq!(history_delete(&f.session, id), Err(SessionError::NotFound));
    }

    #[test]
    fn history_group_hidden_from_sidebar_search_and_credential_candidates() {
        let f = fx();
        record(&f, "hidden-pw", "https://example.com");
        let group_id = GENERATOR_HISTORY_GROUP_UUID.simple().to_string();
        f.session
            .with_db(|db| {
                let bar = sidebar(db);
                assert!(bar.groups.iter().all(|g| g.id != group_id));
                for category in [Category::All, Category::Security] {
                    let q = EntryQuery {
                        scope: Scope::Category { category },
                        search: String::new(),
                        sort: SortKey::Title,
                    };
                    assert!(list_entries(db, &q, true).is_empty());
                }
                let q = EntryQuery {
                    scope: Scope::Category {
                        category: Category::All,
                    },
                    search: "example".into(),
                    sort: SortKey::Title,
                };
                assert!(list_entries(db, &q, true).is_empty());
                assert!(credential_candidates(db, "https://example.com").is_empty());
            })
            .unwrap();
    }

    #[test]
    fn record_history_stores_value_for_generator_copy() {
        let f = fx();
        let id = record(&f, "copied-value", "https://copy.example.org");
        assert_eq!(
            f.session.with_db(|db| history_reveal(db, id)).unwrap(),
            Ok("copied-value".to_owned())
        );
    }

    #[test]
    fn without_record_url_vault_untouched() {
        let f = fx();
        let before = f.session.status().save.state;
        generate_password(Some(&f.session), &req(opts())).unwrap();
        assert_eq!(f.session.status().save.state, before);
        assert_eq!(f.session.status().save.state, SaveState::Idle);
        f.session
            .with_db(|db| assert!(db.find_group(GENERATOR_HISTORY_GROUP_UUID).is_none()))
            .unwrap();
        f.session.lock(crate::session::LockReason::Manual);
        assert!(generate_password(Some(&f.session), &req(opts())).is_ok());
        assert!(generate_password(None, &req(opts())).is_ok());
    }

    #[test]
    fn recording_without_unlocked_session_returns_no_value() {
        let with_url = GenerateRequest {
            record_url: Some("https://example.com".into()),
            ..req(opts())
        };
        assert_eq!(
            generate_password(None, &with_url).unwrap_err(),
            SessionError::NoVault
        );
        let f = fx();
        f.session.lock(crate::session::LockReason::Manual);
        assert_eq!(
            generate_password(Some(&f.session), &with_url).unwrap_err(),
            SessionError::Locked
        );
    }
}
