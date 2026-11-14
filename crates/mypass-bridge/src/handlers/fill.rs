//! `fillCredential`, `fillAddress`, `getTotp`.
//!
//! The TOTP code is computed from the entry's own `otp` parameters (digits, period, algorithm), so
//! an `otpauth://` entry with 8 digits or SHA-256 fills the right code. The earlier app hard-coded
//! 6 digits, 30 seconds and SHA-1.
//!
//! Secrets leave the session only inside the response: they are copied once, into `Zeroizing`
//! buffers, under the session lock, and moved into the response without further copies here. The
//! response holds plain `String`s and the server serialises it into a buffer that is not wiped, so
//! this limits the copies; it does not promise erasure. Nothing here is logged.

use std::time::UNIX_EPOCH;

use mypass_core::entry_ext::{EntryExt, EntryType};
use mypass_core::matcher::credential_candidates;
use mypass_kdbx::Entry;
use mypass_session::fields::EntryFields;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::protocol::Response;
use crate::service::Ctx;
use crate::views::{address, error, parse_wire_id, session_error_response};

const NOT_FOUND: &str = "entry not found or domain mismatch";

fn unix_secs(ctx: &Ctx<'_>) -> u64 {
    ctx.now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The live code of `entry` at `now`, or `None` without a usable secret.
fn live_totp(entry: &Entry, now: u64) -> Option<Zeroizing<String>> {
    entry
        .totp()
        .and_then(|t| t.code_at(now).ok())
        .map(Zeroizing::new)
}

/// Moves a secret out of its zeroizing wrapper without copying it: the wrapper keeps an empty
/// string, the caller owns the buffer.
fn take(mut s: Zeroizing<String>) -> String {
    std::mem::take(&mut *s)
}

/// Records the use of an entry: access time and count, never marks the vault dirty.
fn touch(ctx: &Ctx<'_>, id: Uuid) -> Result<(), Response> {
    ctx.deps
        .session
        .touch(|db, now| db.mark_accessed(id, now))
        .map(|_| ())
        .map_err(|e| session_error_response(&e))
}

struct Filled {
    id: Uuid,
    username: Zeroizing<String>,
    password: Zeroizing<String>,
    totp: Option<Zeroizing<String>>,
    urls: Vec<String>,
}

pub(crate) fn fill_credential(ctx: &Ctx<'_>, id: &str, url: &str) -> Response {
    let wanted = parse_wire_id(id);
    let now = unix_secs(ctx);
    let found = ctx.deps.session.with_db(|db| {
        credential_candidates(db, url)
            .into_iter()
            .find(|e| Some(e.uuid) == wanted)
            .map(|e| Filled {
                id: e.uuid,
                username: Zeroizing::new(e.field("UserName").unwrap_or("").to_owned()),
                password: Zeroizing::new(e.field("Password").unwrap_or("").to_owned()),
                totp: live_totp(e, now),
                urls: e.urls(),
            })
    });
    let filled = match found {
        Ok(Some(f)) => f,
        Ok(None) => return error(NOT_FOUND),
        Err(_) => return Response::Locked,
    };
    if let Err(r) = touch(ctx, filled.id) {
        return r;
    }
    ctx.deps.ui.credential_filled(filled.id);
    Response::Credential {
        username: take(filled.username),
        password: take(filled.password),
        totp: filled.totp.map(take),
        urls: filled.urls,
    }
}

/// `true` for an `http(s)` URL with a host and no userinfo.
fn is_website_url(url: &str) -> bool {
    url::Url::parse(url.trim()).is_ok_and(|u| {
        matches!(u.scheme(), "http" | "https")
            && u.host_str().is_some_and(|h| !h.is_empty())
            && u.username().is_empty()
            && u.password().is_none()
    })
}

pub(crate) fn fill_address(ctx: &Ctx<'_>, id: &str, url: &str) -> Response {
    if !is_website_url(url) {
        return error("Address autofill requires a website URL");
    }
    let wanted = parse_wire_id(id);
    let found = ctx.deps.session.with_db(|db| {
        wanted
            .and_then(|id| db.find_entry(id))
            .filter(|e| e.entry_type() == EntryType::Address && !db.is_in_recycle_bin(e.uuid))
            .map(|e| (e.uuid, address(e)))
    });
    let (uuid, address) = match found {
        Ok(Some(f)) => f,
        Ok(None) => return error("Address not found"),
        Err(_) => return Response::Locked,
    };
    // Addresses are reusable across websites; the extension pins the selection to one form.
    if let Err(r) = touch(ctx, uuid) {
        return r;
    }
    Response::Address {
        address: Box::new(address),
    }
}

pub(crate) fn get_totp(ctx: &Ctx<'_>, id: &str, url: &str) -> Response {
    let wanted = parse_wire_id(id);
    let now = unix_secs(ctx);
    let found = ctx.deps.session.with_db(|db| {
        credential_candidates(db, url)
            .into_iter()
            .find(|e| Some(e.uuid) == wanted)
            .map(|e| live_totp(e, now))
    });
    match found {
        Ok(Some(Some(code))) => Response::Totp { code: take(code) },
        Ok(Some(None)) => error("entry has no TOTP secret"),
        Ok(None) => error(NOT_FOUND),
        Err(_) => Response::Locked,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::passkeys::NotSupportedPasskeys;
    use crate::protocol::Request;
    use crate::testing::{fx_with_db, resets_timer, sample_database, sample_fx, send, Fx, Ids};
    use crate::views::wire_id;
    use mypass_core::TotpConfig;
    use mypass_session::session::SaveState;
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};

    const GITHUB: &str = "https://github.com/login";
    const OTPAUTH: &str = "otpauth://totp/x?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&digits=8&period=60&algorithm=SHA256";
    const PLAIN: &str = "JBSWY3DPEHPK3PXP";

    fn fill(id: Uuid, url: &'static str) -> impl FnOnce(String, Option<String>) -> Request {
        move |token, pairing| Request::FillCredential {
            token,
            id: wire_id(id),
            url: url.into(),
            pairing,
        }
    }

    fn fill_id(
        id: &'static str,
        url: &'static str,
    ) -> impl FnOnce(String, Option<String>) -> Request {
        move |token, pairing| Request::FillCredential {
            token,
            id: id.into(),
            url: url.into(),
            pairing,
        }
    }

    fn totp(id: Uuid, url: &'static str) -> impl FnOnce(String, Option<String>) -> Request {
        move |token, pairing| Request::GetTotp {
            token,
            id: wire_id(id),
            url: url.into(),
            pairing,
        }
    }

    fn address_req(id: Uuid, url: &'static str) -> impl FnOnce(String, Option<String>) -> Request {
        move |token, pairing| Request::FillAddress {
            token,
            id: wire_id(id),
            url: url.into(),
            pairing,
        }
    }

    /// The sample vault with entry A's `otp` replaced and the wall clock at `t` seconds.
    fn fx_otp(otp: &str, t: u64) -> (Fx, Ids) {
        let (mut db, ids) = sample_database();
        db.find_entry_mut(ids.a)
            .unwrap()
            .set_field("otp", otp, true);
        let f = fx_with_db(Arc::new(NotSupportedPasskeys), db);
        f.sched.set_wall_clock(UNIX_EPOCH + Duration::from_secs(t));
        (f, ids)
    }

    fn expected(otp: &str, t: u64) -> String {
        TotpConfig::parse(otp).unwrap().code_at(t).unwrap()
    }

    fn old_hardcoded(otp: &str, t: u64) -> String {
        let secret = TotpConfig::parse(otp).unwrap().secret;
        let c = TotpConfig {
            secret,
            digits: 6,
            period: 30,
            algorithm: mypass_core::totp::TotpAlgorithm::Sha1,
        };
        c.code_at(t).unwrap()
    }

    fn credential(r: &Response) -> (&str, &str, Option<&str>, &[String]) {
        let Response::Credential {
            username,
            password,
            totp,
            urls,
        } = r
        else {
            panic!("expected Credential, got {r:?}");
        };
        (username, password, totp.as_deref(), urls)
    }

    fn msg(r: &Response) -> &str {
        let Response::Error { message } = r else {
            panic!("expected Error, got {r:?}");
        };
        message
    }

    #[test]
    fn fill_credential_returns_live_totp_with_otpauth_digits() {
        let (f, ids) = fx_otp(OTPAUTH, 59);
        let r = send(&f, fill(ids.a, GITHUB));
        let (user, pass, code, _) = credential(&r);
        assert_eq!((user, pass), ("alice", crate::testing::A_PASSWORD));
        let code = code.unwrap();
        assert_eq!(code.len(), 8);
        assert_eq!(code, expected(OTPAUTH, 59));

        let (f, ids) = fx_otp(PLAIN, 59);
        let r = send(&f, fill(ids.a, GITHUB));
        let code = credential(&r).2.unwrap();
        assert_eq!(code.len(), 6);
        assert_eq!(code, expected(PLAIN, 59));
        assert_eq!(code, old_hardcoded(PLAIN, 59));
    }

    #[test]
    fn get_totp_honours_digits_period_algorithm() {
        for (otp, t) in [(OTPAUTH, 59), (OTPAUTH, 61), (PLAIN, 59)] {
            let (f, ids) = fx_otp(otp, t);
            let r = send(&f, totp(ids.a, GITHUB));
            let Response::Totp { code } = r else {
                panic!("expected Totp");
            };
            assert_eq!(code, expected(otp, t));
        }
        // The old 6 digit, 30 s, SHA-1 answer is not the answer for an otpauth entry.
        let (f, ids) = fx_otp(OTPAUTH, 59);
        let Response::Totp { code } = send(&f, totp(ids.a, GITHUB)) else {
            panic!("expected Totp");
        };
        assert_ne!(code, old_hardcoded(OTPAUTH, 59));
        assert_eq!(code.len(), 8);
    }

    #[test]
    fn fill_credential_domain_mismatch_error() {
        let (f, ids) = sample_fx();
        let r = send(&f, fill(ids.a, "https://evil.example"));
        assert_eq!(msg(&r), "entry not found or domain mismatch");
        assert_eq!(
            msg(&send(&f, totp(ids.a, "https://evil.example"))),
            "entry not found or domain mismatch"
        );
    }

    #[test]
    fn fill_credential_wrong_id_error() {
        let (f, _) = sample_fx();
        for id in ["", "not-an-id", "00000000-0000-0000-0000-000000000000"] {
            let r = send(&f, fill_id(id, GITHUB));
            assert_eq!(msg(&r), "entry not found or domain mismatch");
        }
    }

    #[test]
    fn fill_credential_urls_include_additional_urls_and_omit_when_empty() {
        let (f, ids) = sample_fx();
        let r = send(&f, fill(ids.a, GITHUB));
        assert_eq!(
            credential(&r).3,
            ["https://github.com", "https://gist.github.com"]
        );
        let json = serde_json::to_value(&Response::Credential {
            username: "u".into(),
            password: "p".into(),
            totp: None,
            urls: vec![],
        })
        .unwrap();
        assert!(json.get("urls").is_none());
        assert!(json.get("totp").is_none());
    }

    #[test]
    fn fill_credential_touches_access_time_without_marking_dirty() {
        let (f, ids) = sample_fx();
        let usage = |f: &Fx| {
            f.session
                .with_db(|db| {
                    let e = db.find_entry(ids.a).unwrap();
                    (e.usage_count, e.accessed)
                })
                .unwrap()
        };
        let (before_count, before_time) = usage(&f);
        send(&f, fill(ids.a, GITHUB));
        let (count, time) = usage(&f);
        assert_eq!(count, before_count + 1);
        // The session clock is the injected one, so the stamp is exactly its wall time.
        assert_ne!(time, before_time);
        assert_eq!(time, mypass_kdbx::Timestamp(1_700_000_000));
        assert_eq!(f.session.status().save.state, SaveState::Idle);
    }

    #[test]
    fn fill_credential_calls_ui_credential_filled_once() {
        let (f, ids) = sample_fx();
        send(&f, fill(ids.a, GITHUB));
        assert_eq!(f.ui.calls(), ["credential_filled"]);
    }

    #[test]
    fn get_totp_does_not_touch_or_notify() {
        let (f, ids) = sample_fx();
        let before = f
            .session
            .with_db(|db| db.find_entry(ids.a).unwrap().usage_count)
            .unwrap();
        let r = send(&f, totp(ids.a, GITHUB));
        assert!(matches!(r, Response::Totp { .. }));
        let after = f
            .session
            .with_db(|db| db.find_entry(ids.a).unwrap().usage_count)
            .unwrap();
        assert_eq!(before, after);
        assert!(f.ui.calls().is_empty());
    }

    #[test]
    fn get_totp_without_secret_error() {
        let (mut db, ids) = sample_database();
        db.find_entry_mut(ids.a).unwrap().remove_field("otp");
        let f = fx_with_db(Arc::new(NotSupportedPasskeys), db);
        let r = send(&f, totp(ids.a, GITHUB));
        assert_eq!(msg(&r), "entry has no TOTP secret");
        // An unparseable secret is the same answer, and fill serves no code.
        let (f, ids) = fx_otp("not base32 !!", 59);
        assert_eq!(
            msg(&send(&f, totp(ids.a, GITHUB))),
            "entry has no TOTP secret"
        );
        let r = send(&f, fill(ids.a, GITHUB));
        assert_eq!(credential(&r).2, None);
    }

    #[test]
    fn fill_credential_never_serves_files_or_bin_entries() {
        let (f, ids) = sample_fx();
        for hidden in [ids.f, ids.g, ids.h, ids.c, ids.never] {
            let r = send(&f, fill(hidden, GITHUB));
            assert_eq!(msg(&r), "entry not found or domain mismatch");
            let r = send(&f, totp(hidden, GITHUB));
            assert_eq!(msg(&r), "entry not found or domain mismatch");
        }
        assert!(f.ui.calls().is_empty());
    }

    #[test]
    fn fill_address_requires_http_url_without_userinfo() {
        let (f, ids) = sample_fx();
        for bad in [
            "ftp://x",
            "https://u:p@x.com",
            "https:///",
            "",
            "x.com",
            "https://:p@x.com",
        ] {
            let r = send(&f, address_req(ids.c, bad));
            assert_eq!(msg(&r), "Address autofill requires a website URL", "{bad}");
        }
        assert!(matches!(
            send(&f, address_req(ids.c, "https://x.com")),
            Response::Address { .. }
        ));
        assert!(matches!(
            send(&f, address_req(ids.c, "http://x.com/form")),
            Response::Address { .. }
        ));
    }

    #[test]
    fn fill_address_wrong_type_or_bin_is_address_not_found() {
        let (mut db, ids) = sample_database();
        let now = mypass_kdbx::Timestamp::now();
        let mut extra = Entry::new(now);
        extra.set_field("MyPass.EntryType", "Address", false);
        let binned = extra.uuid;
        let root = db.root_id();
        db.add_entry(root, extra, now).unwrap();
        db.delete_entry(binned, now).unwrap();
        let f = fx_with_db(Arc::new(NotSupportedPasskeys), db);
        for id in [ids.a, ids.d, ids.f, binned] {
            let r = send(&f, address_req(id, "https://x.com"));
            assert_eq!(msg(&r), "Address not found");
        }
        let r = send(&f, |token, pairing| Request::FillAddress {
            token,
            id: "garbage".into(),
            url: "https://x.com".into(),
            pairing,
        });
        assert_eq!(msg(&r), "Address not found");
    }

    #[test]
    fn fill_address_reusable_across_sites() {
        let (f, ids) = sample_fx();
        for url in ["https://one.example/a", "https://two.example/b"] {
            assert!(matches!(
                send(&f, address_req(ids.c, url)),
                Response::Address { .. }
            ));
        }
    }

    #[test]
    fn fill_address_maps_all_14_fields_plus_id() {
        let (mut db, ids) = sample_database();
        let keys = [
            ("label", "Work"),
            ("fullName", "Alice Example"),
            ("givenName", "Alice"),
            ("familyName", "Example"),
            ("organization", "Acme"),
            ("addressLine1", "1 Main St"),
            ("addressLine2", "Flat 2"),
            ("city", "Kyiv"),
            ("region", "Kyiv Oblast"),
            ("postalCode", "01001"),
            ("countryCode", "UA"),
            ("countryName", "Ukraine"),
            ("phone", "+380000000"),
            ("email", "a@example.org"),
        ];
        let entry = db.find_entry_mut(ids.c).unwrap();
        for (k, v) in keys {
            entry.set_field(&format!("MyPass.Address.{k}"), v, true);
        }
        let f = fx_with_db(Arc::new(NotSupportedPasskeys), db);
        let r = send(&f, address_req(ids.c, "https://x.com"));
        let json = serde_json::to_value(&r).unwrap();
        let a = &json["address"];
        assert_eq!(a["id"], wire_id(ids.c));
        for (k, v) in keys {
            assert_eq!(a[k], v, "{k}");
        }
        assert_eq!(a.as_object().unwrap().len(), 15);
        // Touched, not dirty.
        assert_eq!(f.session.status().save.state, SaveState::Idle);
    }

    #[test]
    fn served_fill_resets_the_timer_and_an_unknown_id_does_not() {
        let (f, ids) = sample_fx();
        assert!(resets_timer(&f, || {
            assert!(matches!(
                send(&f, fill(ids.a, GITHUB)),
                Response::Credential { .. }
            ));
        }));
        let (f, _) = sample_fx();
        assert!(!resets_timer(&f, || {
            assert!(matches!(
                send(&f, fill_id("00000000-0000-0000-0000-000000000000", GITHUB)),
                Response::Error { .. }
            ));
        }));
    }

    #[test]
    fn fill_handlers_answer_locked_when_locked() {
        let (f, ids) = sample_fx();
        f.session.lock(mypass_session::session::LockReason::Manual);
        for r in [
            send(&f, fill(ids.a, GITHUB)),
            send(&f, totp(ids.a, GITHUB)),
            send(&f, address_req(ids.c, "https://x.com")),
        ] {
            assert_eq!(r, Response::Locked);
        }
    }
}
