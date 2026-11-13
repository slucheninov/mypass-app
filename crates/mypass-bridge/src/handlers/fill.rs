//! `fillCredential`, `fillAddress`, `getTotp` (Task 12).

use super::not_implemented;
use crate::protocol::Response;
use crate::service::Ctx;

pub(crate) fn fill_credential(_ctx: &Ctx<'_>, _id: &str, _url: &str) -> Response {
    not_implemented()
}

pub(crate) fn fill_address(_ctx: &Ctx<'_>, _id: &str, _url: &str) -> Response {
    not_implemented()
}

pub(crate) fn get_totp(_ctx: &Ctx<'_>, _id: &str, _url: &str) -> Response {
    not_implemented()
}
