//! `getCredentials`, `getFavorites`, `listEntries`, `searchEntries` (Task 11).

use super::not_implemented;
use crate::protocol::Response;
use crate::service::Ctx;

pub(crate) fn get_credentials(_ctx: &Ctx<'_>, _url: &str) -> Response {
    not_implemented()
}

pub(crate) fn get_favorites(_ctx: &Ctx<'_>) -> Response {
    not_implemented()
}

pub(crate) fn list_entries(_ctx: &Ctx<'_>, _category: &str) -> Response {
    not_implemented()
}

pub(crate) fn search_entries(_ctx: &Ctx<'_>, _query: &str) -> Response {
    not_implemented()
}
