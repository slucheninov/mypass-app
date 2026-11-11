//! The vault session, settings, scheduling and the command facade behind the Tauri shell.
//!
//! This crate has no Tauri dependency, so it is testable without a window server. Every timing
//! rule runs through [`scheduler::Scheduler`]; tests drive it with [`scheduler::ManualScheduler`].

pub mod scheduler;
pub mod settings;
pub mod shortcut;
