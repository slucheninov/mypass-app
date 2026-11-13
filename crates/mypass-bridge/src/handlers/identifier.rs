//! `addEntryIdentifier` (Task 13).

use super::not_implemented;
use crate::protocol::Response;
use crate::service::Ctx;

pub(crate) fn add_entry_identifier(
    _ctx: &Ctx<'_>,
    _id: &str,
    _url: &str,
    _value: &str,
    _field_key: &str,
) -> Response {
    not_implemented()
}
