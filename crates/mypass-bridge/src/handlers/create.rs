//! `createEntry` (Task 13).

use super::not_implemented;
use crate::protocol::Response;
use crate::service::Ctx;

/// The fields of a `createEntry` request.
#[allow(dead_code)] // read by the handler of Task 13
pub(crate) struct CreateEntryArgs<'a> {
    pub entry_type: &'a str,
    pub title: &'a str,
    pub username: &'a str,
    pub password: &'a str,
    pub url: &'a str,
    pub card_number: &'a str,
    pub cardholder_name: &'a str,
    pub expiry: &'a str,
    pub cvv: &'a str,
    pub notes: &'a str,
    pub favorite: bool,
    pub match_mode: &'a str,
    pub otp: &'a str,
}

pub(crate) fn create_entry(_ctx: &Ctx<'_>, _args: &CreateEntryArgs<'_>) -> Response {
    not_implemented()
}
