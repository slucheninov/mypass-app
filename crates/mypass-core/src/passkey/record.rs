//! The `KPEX_PASSKEY_*` entry fields (a contract with KeePassXC and the earlier Swift app, so the
//! names never change) and the one definition of "this entry carries a usable passkey".

use super::b64url;
use mypass_kdbx::Entry;

/// Prefix shared by every passkey field.
pub const PASSKEY_PREFIX: &str = "KPEX_PASSKEY_";
/// Credential id, base64url.
pub const PASSKEY_CREDENTIAL_ID: &str = "KPEX_PASSKEY_CREDENTIAL_ID";
/// PKCS#8 PEM private key (protected).
pub const PASSKEY_PRIVATE_KEY_PEM: &str = "KPEX_PASSKEY_PRIVATE_KEY_PEM";
/// Relying party id.
pub const PASSKEY_RELYING_PARTY: &str = "KPEX_PASSKEY_RELYING_PARTY";
/// Account user name shown for the passkey.
pub const PASSKEY_USERNAME: &str = "KPEX_PASSKEY_USERNAME";
/// MyPass's own addition (an ISO 8601 timestamp); KeePassXC ignores it.
pub const PASSKEY_CREATED: &str = "KPEX_PASSKEY_CREATED";
/// User handle, base64url.
pub const PASSKEY_USER_HANDLE: &str = "KPEX_PASSKEY_USER_HANDLE";
/// Backup-eligible flag.
pub const PASSKEY_FLAG_BE: &str = "KPEX_PASSKEY_FLAG_BE";
/// Backup-state flag.
pub const PASSKEY_FLAG_BS: &str = "KPEX_PASSKEY_FLAG_BS";

/// A usable passkey: the credential id decodes (lenient base64url) to at least one byte and the
/// private key PEM is non-empty. The PEM is deliberately not parsed here, so listing stays cheap
/// and an unreadable key still shows as a passkey the user can inspect or delete.
pub fn has_passkey(entry: &Entry) -> bool {
    let id_ok = entry
        .field(PASSKEY_CREDENTIAL_ID)
        .and_then(b64url::decode)
        .is_some_and(|id| !id.is_empty());
    let pem_ok = entry
        .field(PASSKEY_PRIVATE_KEY_PEM)
        .is_some_and(|v| !v.is_empty());
    id_ok && pem_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(fields: &[(&str, &str)]) -> Entry {
        let mut e = Entry::new(mypass_kdbx::Timestamp(0));
        for (k, v) in fields {
            e.set_field(k, v, false);
        }
        e
    }

    #[test]
    fn predicate_cases() {
        let cases: Vec<(Vec<(&str, &str)>, bool)> = vec![
            (
                vec![
                    (PASSKEY_CREDENTIAL_ID, "abc"),
                    (PASSKEY_PRIVATE_KEY_PEM, "pem"),
                ],
                true,
            ),
            (vec![(PASSKEY_CREDENTIAL_ID, "abc")], false),
            (vec![(PASSKEY_PRIVATE_KEY_PEM, "pem")], false),
            (
                vec![(PASSKEY_CREDENTIAL_ID, ""), (PASSKEY_PRIVATE_KEY_PEM, "")],
                false,
            ),
            (
                vec![
                    (PASSKEY_CREDENTIAL_ID, "!!"),
                    (PASSKEY_PRIVATE_KEY_PEM, "pem"),
                ],
                false,
            ),
            (vec![], false),
        ];
        for (fields, expected) in cases {
            assert_eq!(has_passkey(&entry(&fields)), expected, "{fields:?}");
        }
    }
}
