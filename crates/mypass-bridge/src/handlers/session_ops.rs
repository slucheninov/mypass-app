//! Session, proposal, pairing and diagnostics requests (Task 14).

use super::not_implemented;
use crate::protocol::{BrowserDiagnosticsReport, Response};
use crate::service::Ctx;

pub(crate) fn propose_save_login(
    _ctx: &Ctx<'_>,
    _url: &str,
    _username: &str,
    _password: &str,
) -> Response {
    not_implemented()
}

pub(crate) fn propose_save_totp(
    _ctx: &Ctx<'_>,
    _url: &str,
    _otpauth_or_secret: &str,
    _entry_id: Option<&str>,
) -> Response {
    not_implemented()
}

pub(crate) fn start_pairing(_ctx: &Ctx<'_>, _browser: Option<&str>) -> Response {
    not_implemented()
}

pub(crate) fn complete_pairing(_ctx: &Ctx<'_>, _code: &str) -> Response {
    not_implemented()
}

pub(crate) fn request_unlock(_ctx: &Ctx<'_>) -> Response {
    not_implemented()
}

pub(crate) fn lock_vault(_ctx: &Ctx<'_>) -> Response {
    not_implemented()
}

pub(crate) fn open_app(_ctx: &Ctx<'_>) -> Response {
    not_implemented()
}

pub(crate) fn open_browser_settings(_ctx: &Ctx<'_>) -> Response {
    not_implemented()
}

pub(crate) fn report_browser_diagnostics(
    _ctx: &Ctx<'_>,
    _report: &BrowserDiagnosticsReport,
    _pairing: Option<&str>,
) -> Response {
    not_implemented()
}
