//! The WebAuthn errors the extension rethrows into the page: a completion with no response and
//! a `DOMException` name and message.

use crate::protocol::{PasskeyErrorDetails, Response};

/// `NotAllowedError`: the name for every refusal the page must not learn the reason of.
pub(crate) const NOT_ALLOWED: &str = "NotAllowedError";

fn details(name: &str, message: &str) -> Option<PasskeyErrorDetails> {
    Some(PasskeyErrorDetails {
        name: name.to_owned(),
        message: message.to_owned(),
    })
}

/// A failed create ceremony (`passkeyCreateComplete` with `responseJson` absent).
pub(crate) fn create_error(id: i64, name: &str, message: &str) -> Response {
    Response::PasskeyCreateComplete {
        request_id: id,
        response_json: None,
        error: details(name, message),
    }
}

/// A failed get ceremony (`passkeyGetComplete` with `responseJson` absent).
pub(crate) fn get_error(id: i64, name: &str, message: &str) -> Response {
    Response::PasskeyGetComplete {
        request_id: id,
        response_json: None,
        error: details(name, message),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn errors_serialise_without_response_json_and_are_not_served() {
        let c = create_error(3, NOT_ALLOWED, "No pending request");
        assert_eq!(
            serde_json::to_value(&c).unwrap(),
            json!({"type": "passkeyCreateComplete", "requestId": 3,
                   "error": {"name": "NotAllowedError", "message": "No pending request"}})
        );
        let g = get_error(-1, "SecurityError", "x");
        assert_eq!(
            serde_json::to_value(&g).unwrap(),
            json!({"type": "passkeyGetComplete", "requestId": -1,
                   "error": {"name": "SecurityError", "message": "x"}})
        );
        assert!(!c.was_served());
        assert!(!g.was_served());
    }
}
