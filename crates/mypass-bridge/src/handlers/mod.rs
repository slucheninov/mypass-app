//! One module per group of requests. Each handler takes the per-request [`Ctx`](crate::service::Ctx)
//! and its own fields; the gates have already run.
//!
//! Until the tasks that own them land, the handlers here are stubs answering the placeholder error.

use crate::protocol::Response;

pub(crate) mod browse;
pub(crate) mod create;
pub(crate) mod fill;
pub(crate) mod generator;
pub(crate) mod identifier;
pub(crate) mod session_ops;

/// The answer of a handler that a later task replaces.
pub(crate) fn not_implemented() -> Response {
    Response::Error {
        message: "not implemented".to_owned(),
    }
}
