#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)] // test-only

use super::pending::tests::{pc, pc_for, pg, pg_for};
use super::pending::PENDING_TTL;
use super::*;
use crate::testing::*;
use mypass_session::session::{LockReason, SessionError};
use mypass_session::testing::fast_database;
use secrecy::SecretString;
use std::time::Duration;

fn ctx<'a>(f: &'a Fx, settings: &'a SettingsStore, unlocked: bool) -> PasskeyContext<'a> {
    PasskeyContext {
        session: &f.session,
        settings,
        unlocked,
    }
}

fn set_toggle(settings: &SettingsStore, on: bool) {
    settings
        .update(SettingsPatch {
            passkey_proxy_enabled: Some(on),
            ..SettingsPatch::default()
        })
        .unwrap();
}

fn set_enabled_request(f: &Fx, enabled: bool) -> Request {
    Request::SetPasskeyProxyEnabled {
        token: "tok".into(),
        enabled,
        pairing: Some(f.secret.clone()),
    }
}

/// Fills both tables through the service API.
fn fill(svc: &PasskeyService, ids: &[i64]) {
    for &id in ids {
        assert!(svc.put_create(svc.epoch(), id, pc()));
        assert!(svc.put_get(svc.epoch(), id, pg()));
    }
}

#[test]
fn status_reports_enabled_and_attach_allowed_from_lock_state_and_toggle() {
    let (f, svc) = passkey_fx();
    let status = named("getPasskeyProxyStatus", Some(&f.secret));
    for (unlocked, on, attach) in [
        (true, true, true),
        (true, false, false),
        (false, true, false),
        (false, false, false),
    ] {
        set_toggle(&f.settings, on);
        assert_eq!(
            svc.handle(&ctx(&f, &f.settings, unlocked), &status),
            Response::PasskeyProxyStatus {
                unlocked,
                paired: true,
                attach_allowed: attach,
                enabled: on,
            },
            "unlocked {unlocked}, enabled {on}"
        );
    }
    // The same through the gates: locked and enabled reports `enabled` so the extension resyncs.
    set_toggle(&f.settings, true);
    f.session.lock(LockReason::Manual);
    assert_eq!(
        f.svc
            .handle(named("getPasskeyProxyStatus", Some(&f.secret))),
        Response::PasskeyProxyStatus {
            unlocked: false,
            paired: true,
            attach_allowed: false,
            enabled: true,
        }
    );
}

#[test]
fn status_is_enabled_when_toggle_never_touched() {
    let (f, _svc) = passkey_fx();
    assert_eq!(
        f.svc
            .handle(named("getPasskeyProxyStatus", Some(&f.secret))),
        Response::PasskeyProxyStatus {
            unlocked: true,
            paired: true,
            attach_allowed: true,
            enabled: true,
        }
    );
}

#[test]
fn set_enabled_persists_and_returns_stored_value() {
    let (f, svc) = passkey_fx();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    let store = SettingsStore::load(path.clone());
    assert_eq!(
        svc.handle(&ctx(&f, &store, true), &set_enabled_request(&f, false)),
        Response::PasskeyProxyEnabled { enabled: false }
    );
    assert!(
        !SettingsStore::load(path.clone())
            .get()
            .passkey_proxy_enabled
    );
    assert_eq!(
        svc.handle(&ctx(&f, &store, true), &set_enabled_request(&f, true)),
        Response::PasskeyProxyEnabled { enabled: true }
    );
    assert!(SettingsStore::load(path).get().passkey_proxy_enabled);

    // A store that cannot write (its file sits under a regular file, so the read at load fails
    // and the store runs degraded): the new value is still in force in memory for this run, and
    // the answer reports exactly that stored value.
    let blocker = dir.path().join("not-a-dir");
    std::fs::write(&blocker, b"x").unwrap();
    let broken = SettingsStore::load(blocker.join("settings.json"));
    assert!(broken.is_degraded());
    assert_eq!(
        svc.handle(&ctx(&f, &broken, true), &set_enabled_request(&f, false)),
        Response::PasskeyProxyEnabled { enabled: false }
    );
    assert!(!broken.get().passkey_proxy_enabled);
}

#[test]
fn set_enabled_notifies_ui_once_per_change() {
    let (f, _svc) = passkey_fx();
    for on in [true, false, false, true, true] {
        assert_eq!(
            f.svc.handle(set_enabled_request(&f, on)),
            Response::PasskeyProxyEnabled { enabled: on }
        );
    }
    let toggles: Vec<String> =
        f.ui.details()
            .into_iter()
            .filter(|d| d.starts_with("passkey_proxy_changed"))
            .collect();
    assert_eq!(
        toggles,
        ["passkey_proxy_changed false", "passkey_proxy_changed true"]
    );
    assert!(f.settings.get().passkey_proxy_enabled);
}

#[test]
fn turning_the_toggle_off_drops_pending_ceremonies() {
    let (f, svc) = passkey_fx();
    fill(&svc, &[1, 2]);
    f.svc.handle(set_enabled_request(&f, true));
    assert_eq!(svc.pending_len(), (2, 2));
    f.svc.handle(set_enabled_request(&f, false));
    assert_eq!(svc.pending_len(), (0, 0));
}

#[test]
fn cancel_clears_both_tables_and_is_idempotent() {
    let (f, svc) = passkey_fx();
    // `named("passkeyCancel")` carries the negative id -9: ids are the extension's, any i64.
    fill(&svc, &[-9, 4]);
    for _ in 0..2 {
        assert_eq!(
            f.svc.handle(named("passkeyCancel", Some(&f.secret))),
            Response::PasskeyCancelled { request_id: -9 }
        );
        assert_eq!(svc.pending_len(), (1, 1));
    }
    assert!(svc.take_create(-9).is_none());
    assert!(svc.take_get(-9).is_none());
    assert!(svc.take_create(4).is_some());
    assert!(svc.take_get(4).is_some());
    // Cancelling an id that was never pending still answers `PasskeyCancelled`.
    let r = f.svc.handle(Request::PasskeyCancel {
        token: "tok".into(),
        request_id: 12345,
        pairing: Some(f.secret.clone()),
    });
    assert_eq!(r, Response::PasskeyCancelled { request_id: 12345 });
}

#[test]
fn pending_ttl_expiry_and_cap_evict_oldest() {
    let (f, svc) = passkey_fx();
    // Keep the vault unlocked past the TTL: the fixture's one minute auto-lock would clear the
    // table on its own.
    f.settings
        .update(SettingsPatch {
            auto_lock_idle_timeout: Some(mypass_session::settings::IdleTimeout::Never),
            ..SettingsPatch::default()
        })
        .unwrap();
    fill(&svc, &[1]);
    f.sched.advance(PENDING_TTL - Duration::from_secs(1));
    assert!(f.session.is_unlocked());
    assert!(svc.take_create(1).is_some());
    f.sched.advance(Duration::from_secs(1));
    // The deadline is the injected monotonic clock, not the wall clock.
    assert!(svc.take_get(1).is_none());
    for id in 0..20 {
        assert!(svc.put_create(svc.epoch(), id, pc()));
    }
    assert_eq!(svc.pending_len(), (pending::MAX_PENDING_PER_KIND, 0));
    assert!(svc.take_create(3).is_none());
    assert!(svc.take_create(4).is_some());
}

#[test]
fn take_is_single_use() {
    let (_f, svc) = passkey_fx();
    fill(&svc, &[7]);
    assert_eq!(svc.take_create(7), Some(pc()));
    assert_eq!(svc.take_create(7), None);
    assert_eq!(svc.take_get(7), Some(pg()));
    assert_eq!(svc.take_get(7), None);
}

#[test]
fn lock_drops_pending_creates_and_gets() {
    let (f, svc) = passkey_fx();
    fill(&svc, &[1, 2, 3]);
    assert_eq!(svc.pending_len(), (3, 3));
    f.session.lock(LockReason::Manual);
    assert_eq!(svc.pending_len(), (0, 0));
}

#[test]
fn lock_between_vault_read_and_put_drops_the_put() {
    let (f, svc) = passkey_fx();
    let epoch = svc.epoch();
    // A phase-one handler read the vault here; the lock lands before its put.
    f.session.with_db(|_| ()).unwrap();
    f.session.lock(LockReason::Manual);
    assert!(!svc.put_create(epoch, 1, pc()));
    assert!(!svc.put_get(epoch, 1, pg()));
    assert_eq!(svc.pending_len(), (0, 0));
}

#[test]
fn vault_switch_drops_pending() {
    let (f, svc) = passkey_fx();
    fill(&svc, &[1]);
    let stale = svc.epoch();
    // A direct switch to another vault while unlocked: one `StateChanged(Unlocked)`, new path.
    let dir = tempfile::tempdir().unwrap();
    f.session
        .create_vault_with(
            &dir.path().join("other.kdbx"),
            SecretString::from("pw".to_owned()),
            fast_database("Other"),
        )
        .unwrap();
    assert!(f.session.is_unlocked());
    assert_eq!(svc.pending_len(), (0, 0));
    // A confirm after the switch finds nothing ("request expired"); a put started before is
    // refused.
    assert!(svc.take_create(1).is_none());
    assert!(svc.take_get(1).is_none());
    assert!(!svc.put_create(stale, 2, pc()));
}

#[test]
fn status_polling_never_resets_idle_timer_or_unlocks() {
    let (f, _svc) = passkey_fx();
    let poll = || {
        let r = f
            .svc
            .handle(named("getPasskeyProxyStatus", Some(&f.secret)));
        assert!(matches!(r, Response::PasskeyProxyStatus { .. }));
        assert!(r.was_served());
    };
    // Served polls while unlocked: the one minute idle timer still runs out.
    assert!(!resets_timer(&f, poll));
    assert!(!f.session.is_unlocked());
    // Polling a locked vault answers and leaves it locked.
    for _ in 0..3 {
        assert_eq!(
            f.svc
                .handle(named("getPasskeyProxyStatus", Some(&f.secret))),
            Response::PasskeyProxyStatus {
                unlocked: false,
                paired: true,
                attach_allowed: false,
                enabled: true,
            }
        );
        f.sched.advance(Duration::from_secs(30));
    }
    assert!(!f.session.is_unlocked());
}

#[test]
fn create_get_toggle_and_cancel_never_reset_idle_timer() {
    for name in [
        "passkeyCreate",
        "passkeyGet",
        "setPasskeyProxyEnabled",
        "passkeyCancel",
    ] {
        // Confirm and approve are gesture-gated: they reset the timer when served (Tasks 7 and 8
        // pin that), so they are not listed here.
        let (f, _svc) = passkey_fx();
        let request = named(name, Some(&f.secret));
        assert!(
            !resets_timer(&f, || {
                f.svc.handle(request);
            }),
            "{name}"
        );
    }
}

#[test]
fn status_answerable_while_locked_but_requires_pairing() {
    let (f, _svc) = passkey_fx();
    f.session.lock(LockReason::Manual);
    for wrong in [None, Some("not-a-secret")] {
        assert_eq!(
            f.svc.handle(named("getPasskeyProxyStatus", wrong)),
            Response::PairingRequired
        );
    }
    assert!(matches!(
        f.svc
            .handle(named("getPasskeyProxyStatus", Some(&f.secret))),
        Response::PasskeyProxyStatus {
            unlocked: false,
            ..
        }
    ));
    // Every other passkey request is refused while locked, paired or not.
    for name in [
        "setPasskeyProxyEnabled",
        "passkeyCreate",
        "passkeyCreateConfirm",
        "passkeyGet",
        "passkeyGetApprove",
        "passkeyCancel",
    ] {
        assert_eq!(
            f.svc.handle(named(name, Some(&f.secret))),
            Response::Locked,
            "{name}"
        );
    }
}

/// R4: inside the handler the toggle comes first, before any parsing and whatever the lock
/// state the gates saw. Called directly, past the gates.
#[test]
fn toggle_off_refuses_create_and_get_before_parsing() {
    let (f, svc) = passkey_fx();
    set_toggle(&f.settings, false);
    for unlocked in [true, false] {
        // `named` carries `{}` as the request details: parsing would fail differently.
        assert_eq!(
            svc.handle(
                &ctx(&f, &f.settings, unlocked),
                &named("passkeyCreate", Some(&f.secret))
            ),
            errors::create_error(7, "NotAllowedError", TURNED_OFF)
        );
        assert_eq!(
            svc.handle(
                &ctx(&f, &f.settings, unlocked),
                &named("passkeyGet", Some(&f.secret))
            ),
            errors::get_error(7, "NotAllowedError", TURNED_OFF)
        );
    }
}

/// R4: at the service level the lock gate still answers first.
#[test]
fn locked_app_answers_locked_to_passkey_create_even_when_toggle_off() {
    let (f, _svc) = passkey_fx();
    set_toggle(&f.settings, false);
    f.session.lock(LockReason::Manual);
    for name in ["passkeyCreate", "passkeyGet"] {
        assert_eq!(f.svc.handle(named(name, Some(&f.secret))), Response::Locked);
    }
}

#[test]
fn ceremony_arms_answer_a_not_allowed_error_for_bad_or_unknown_ceremonies() {
    let (f, _svc) = passkey_fx();
    // `named` carries `{}` as the request details and an id nothing is pending for.
    for (name, complete_kind) in [
        ("passkeyCreate", "create"),
        ("passkeyCreateConfirm", "create"),
        ("passkeyGet", "get"),
        ("passkeyGetApprove", "get"),
    ] {
        let r = f.svc.handle(named(name, Some(&f.secret)));
        assert!(!r.was_served(), "{name}");
        let ok = match (&r, complete_kind) {
            (Response::PasskeyCreateComplete { error, .. }, "create")
            | (Response::PasskeyGetComplete { error, .. }, "get") => {
                error.as_ref().unwrap().name == "NotAllowedError"
            }
            _ => false,
        };
        assert!(ok, "{name}: {r:?}");
    }
}

/// R7 / review I1: phase two is bound to the vault the ceremony was started on. After a direct
/// switch A -> B (never through Locked) and before the observer clears the table, a confirm can
/// still take A's ceremony; the check inside its `mutate` must refuse and leave B untouched.
#[test]
fn ceremony_taken_after_a_direct_vault_switch_cannot_mutate_the_new_vault() {
    let (f, _observed) = passkey_fx();
    // Not registered with the session: stands for the window before the observer's clear runs.
    let svc = PasskeyService::new(f.sched.clone(), f.ui.clone());
    let epoch = svc.epoch();
    let create = f.session.with_db(pc_for).unwrap();
    let get = f.session.with_db(pg_for).unwrap();
    assert!(svc.put_create(epoch, 1, create));
    assert!(svc.put_get(epoch, 1, get));

    let dir = tempfile::tempdir().unwrap();
    f.session
        .create_vault_with(
            &dir.path().join("b.kdbx"),
            SecretString::from("pw".to_owned()),
            fast_database("Vault B"),
        )
        .unwrap();
    assert!(f.session.is_unlocked());
    let b_before = f.session.with_db(Clone::clone).unwrap();

    let create = svc
        .take_create(1)
        .expect("the clear has not run for this table");
    let written = f.session.mutate(|db, now| {
        if !create.vault_matches(db) {
            return Err(SessionError::InvalidInput("request expired".into()));
        }
        let mut e = mypass_kdbx::Entry::new(now);
        e.set_field("Title", "Passkey from vault A", false);
        let root = db.root_id();
        db.add_entry(root, e, now)
            .map_err(|_| SessionError::InvalidInput("add".into()))?;
        Ok(())
    });
    assert_eq!(
        written,
        Err(SessionError::InvalidInput("request expired".into()))
    );
    let get = svc.take_get(1).unwrap();
    assert!(!f.session.with_db(|db| get.vault_matches(db)).unwrap());
    assert_eq!(f.session.with_db(Clone::clone).unwrap(), b_before);

    // Control: a ceremony started on B matches B.
    let on_b = f.session.with_db(pc_for).unwrap();
    assert!(f.session.with_db(|db| on_b.vault_matches(db)).unwrap());
}

/// Review M1: "off" set in the app (not through the extension) leaves the table alone, so
/// confirm and approve re-check the toggle and drop the ceremony.
#[test]
fn toggle_off_in_the_app_refuses_confirm_and_approve() {
    let (f, svc) = passkey_fx();
    // `named` uses request id 7 for both.
    fill(&svc, &[7]);
    set_toggle(&f.settings, false);
    assert_eq!(svc.pending_len(), (1, 1));
    assert_eq!(
        f.svc.handle(named("passkeyCreateConfirm", Some(&f.secret))),
        errors::create_error(7, "NotAllowedError", TURNED_OFF)
    );
    assert_eq!(
        f.svc.handle(named("passkeyGetApprove", Some(&f.secret))),
        errors::get_error(7, "NotAllowedError", TURNED_OFF)
    );
    assert_eq!(svc.pending_len(), (0, 0));
}
