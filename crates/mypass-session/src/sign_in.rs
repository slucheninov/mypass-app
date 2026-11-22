//! The social sign-in method remembered on a login entry: which "Sign in with ..." button the
//! user clicked on the site last time, recorded by the browser extension and shown (and edited) in
//! the app.
//!
//! Three unprotected custom fields hold it:
//! - `MyPass.SignIn.provider`: one of `google`, `facebook`, `apple`, `microsoft`, `github`;
//! - `MyPass.SignIn.lastUsed`: when the extension last saw the click, RFC 3339 UTC
//!   (`YYYY-MM-DDTHH:MM:SSZ`), absent when the user picked the method in the editor;
//! - `MyPass.SignIn.host`: the lowercase host of the page the click happened on, absent when the
//!   user picked the method in the editor. The extension draws its ring only on exactly this
//!   host, so a sibling site under the same registrable domain never learns the method.
//!
//! All keys are reserved (they have their own UI, never a custom-field row). A provider value
//! this build does not know reads as no sign-in method, and the editor leaves such a value alone
//! unless the user picks a method.

use mypass_kdbx::{Entry, Timestamp};
use serde::{Deserialize, Serialize};

/// The custom field holding the provider.
pub const SIGN_IN_PROVIDER: &str = "MyPass.SignIn.provider";
/// The custom field holding the last use, RFC 3339 UTC.
pub const SIGN_IN_LAST_USED: &str = "MyPass.SignIn.lastUsed";
/// The custom field holding the host the click was recorded on.
pub const SIGN_IN_HOST: &str = "MyPass.SignIn.host";

/// A social sign-in provider MyPass recognises.
#[derive(Serialize, Deserialize, specta::Type, Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[serde(rename_all = "lowercase")]
pub enum SignInProvider {
    Google,
    Facebook,
    Apple,
    Microsoft,
    Github,
}

impl SignInProvider {
    pub const ALL: [Self; 5] = [
        Self::Google,
        Self::Facebook,
        Self::Apple,
        Self::Microsoft,
        Self::Github,
    ];

    /// The stored and wire spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Facebook => "facebook",
            Self::Apple => "apple",
            Self::Microsoft => "microsoft",
            Self::Github => "github",
        }
    }

    /// The exact stored spelling (lowercase, no surrounding space); anything else is `None`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == s)
    }
}

/// The sign-in method stored on an entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SignIn {
    pub provider: SignInProvider,
    /// `None` when the field is missing or not a strict RFC 3339 UTC time.
    pub last_used: Option<Timestamp>,
}

/// The entry's sign-in method, `None` when it has none (or an unknown provider value).
#[must_use]
pub fn sign_in_of(entry: &Entry) -> Option<SignIn> {
    let provider = SignInProvider::parse(entry.field(SIGN_IN_PROVIDER)?)?;
    Some(SignIn {
        provider,
        last_used: entry
            .field(SIGN_IN_LAST_USED)
            .and_then(Timestamp::parse_iso8601),
    })
}

/// The host the entry's sign-in method was recorded on (lowercase), `None` when it has no
/// method, the method was picked in the editor, or the stored value is empty.
#[must_use]
pub fn sign_in_host(entry: &Entry) -> Option<&str> {
    sign_in_of(entry)?;
    entry.field(SIGN_IN_HOST).filter(|h| !h.is_empty())
}

/// Records a use of `provider` on the page host `host` at `now`: all three fields, unprotected.
/// The host is stored lowercase.
pub fn record_sign_in(entry: &mut Entry, provider: SignInProvider, host: &str, now: Timestamp) {
    entry.set_field(SIGN_IN_PROVIDER, provider.as_str(), false);
    match now.to_iso8601() {
        Some(when) => entry.set_field(SIGN_IN_LAST_USED, &when, false),
        None => entry.remove_field(SIGN_IN_LAST_USED),
    }
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() {
        entry.remove_field(SIGN_IN_HOST);
    } else {
        entry.set_field(SIGN_IN_HOST, &host, false);
    }
}

/// Sets the method chosen in the editor. The same provider as stored keeps its last use and
/// host; another one drops them (they belonged to the old method); `None` removes all fields. Returns
/// nothing: the caller compares the whole entry to see whether anything changed.
pub fn set_sign_in_provider(entry: &mut Entry, provider: Option<SignInProvider>) {
    match provider {
        None => {
            // An unknown stored value is left alone: this build cannot show it, so the user
            // cannot have cleared it on purpose.
            if entry
                .field(SIGN_IN_PROVIDER)
                .is_none_or(|v| SignInProvider::parse(v).is_some())
            {
                entry.remove_field(SIGN_IN_PROVIDER);
                entry.remove_field(SIGN_IN_LAST_USED);
                entry.remove_field(SIGN_IN_HOST);
            }
        }
        Some(p) => {
            if sign_in_of(entry).map(|s| s.provider) != Some(p) {
                entry.set_field(SIGN_IN_PROVIDER, p.as_str(), false);
                entry.remove_field(SIGN_IN_LAST_USED);
                entry.remove_field(SIGN_IN_HOST);
            }
        }
    }
}

/// The "most recently used" rank of a login entry: the latest of its last sign-in, its last
/// access and its last modification.
#[must_use]
pub fn last_used_rank(entry: &Entry) -> Timestamp {
    let sign_in = sign_in_of(entry).and_then(|s| s.last_used);
    [Some(entry.accessed), Some(entry.modified), sign_in]
        .into_iter()
        .flatten()
        .max_by_key(|t| t.0)
        .unwrap_or(entry.modified)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // test-only
mod tests {
    use super::*;

    #[test]
    fn provider_spellings_round_trip_and_are_strict() {
        for p in SignInProvider::ALL {
            assert_eq!(SignInProvider::parse(p.as_str()), Some(p));
            assert_eq!(
                serde_json::to_value(p).unwrap(),
                serde_json::json!(p.as_str())
            );
        }
        for bad in ["Google", " google", "twitter", "", "git hub"] {
            assert_eq!(SignInProvider::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn record_writes_both_fields_unprotected_and_reads_back() {
        let mut e = Entry::new(Timestamp(0));
        record_sign_in(
            &mut e,
            SignInProvider::Github,
            "Login.Example.COM",
            Timestamp(1_700_000_000),
        );
        assert_eq!(e.field(SIGN_IN_PROVIDER), Some("github"));
        assert_eq!(e.field(SIGN_IN_LAST_USED), Some("2023-11-14T22:13:20Z"));
        assert_eq!(e.field(SIGN_IN_HOST), Some("login.example.com"));
        assert_eq!(sign_in_host(&e), Some("login.example.com"));
        assert!(e.fields.iter().all(|f| !f.protected));
        assert_eq!(
            sign_in_of(&e),
            Some(SignIn {
                provider: SignInProvider::Github,
                last_used: Some(Timestamp(1_700_000_000)),
            })
        );
    }

    #[test]
    fn unknown_provider_and_bad_date_read_leniently() {
        let mut e = Entry::new(Timestamp(0));
        e.set_field(SIGN_IN_PROVIDER, "twitter", false);
        assert_eq!(sign_in_of(&e), None);
        e.set_field(SIGN_IN_PROVIDER, "apple", false);
        e.set_field(SIGN_IN_LAST_USED, "yesterday", false);
        assert_eq!(
            sign_in_of(&e),
            Some(SignIn {
                provider: SignInProvider::Apple,
                last_used: None,
            })
        );
    }

    #[test]
    fn editor_change_keeps_or_drops_the_date() {
        let mut e = Entry::new(Timestamp(0));
        record_sign_in(
            &mut e,
            SignInProvider::Google,
            "a.example",
            Timestamp(1_700_000_000),
        );
        set_sign_in_provider(&mut e, Some(SignInProvider::Google));
        assert!(e.field(SIGN_IN_LAST_USED).is_some());
        assert_eq!(sign_in_host(&e), Some("a.example"));
        set_sign_in_provider(&mut e, Some(SignInProvider::Apple));
        assert_eq!(e.field(SIGN_IN_PROVIDER), Some("apple"));
        assert_eq!(e.field(SIGN_IN_LAST_USED), None);
        assert_eq!(e.field(SIGN_IN_HOST), None);
        assert_eq!(sign_in_host(&e), None);
        set_sign_in_provider(&mut e, None);
        assert_eq!(e.field(SIGN_IN_PROVIDER), None);
        // An unknown stored value survives a save that did not pick a method.
        e.set_field(SIGN_IN_PROVIDER, "twitter", false);
        set_sign_in_provider(&mut e, None);
        assert_eq!(e.field(SIGN_IN_PROVIDER), Some("twitter"));
        set_sign_in_provider(&mut e, Some(SignInProvider::Microsoft));
        assert_eq!(e.field(SIGN_IN_PROVIDER), Some("microsoft"));
    }

    #[test]
    fn rank_is_the_latest_of_sign_in_access_and_modification() {
        let mut e = Entry::new(Timestamp(100));
        e.accessed = Timestamp(300);
        assert_eq!(last_used_rank(&e), Timestamp(300));
        record_sign_in(&mut e, SignInProvider::Google, "a.example", Timestamp(500));
        assert_eq!(last_used_rank(&e), Timestamp(500));
        e.modified = Timestamp(900);
        assert_eq!(last_used_rank(&e), Timestamp(900));
    }
}
