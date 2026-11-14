//! One module per group of requests. Each handler takes the per-request [`Ctx`](crate::service::Ctx)
//! and its own fields; the gates have already run.

pub(crate) mod browse;
pub(crate) mod create;
pub(crate) mod fill;
pub(crate) mod generator;
pub(crate) mod identifier;
pub(crate) mod session_ops;
pub(crate) mod text;
