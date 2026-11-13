//! `generatePassword` and the generated-password history (Task 15).

use super::not_implemented;
use crate::protocol::Response;
use crate::service::Ctx;

pub(crate) fn generate_password(_ctx: &Ctx<'_>, _domain: &str) -> Response {
    not_implemented()
}

pub(crate) fn record_generated_password(
    _ctx: &Ctx<'_>,
    _password: &str,
    _url: &str,
    _created_at: Option<f64>,
) -> Response {
    not_implemented()
}

pub(crate) fn get_generated_password_history(_ctx: &Ctx<'_>) -> Response {
    not_implemented()
}

pub(crate) fn delete_generated_password(_ctx: &Ctx<'_>, _id: &str) -> Response {
    not_implemented()
}
