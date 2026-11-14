//! Vault sync (stage 5): the secondary-folder backends and sync engine, the folder watcher and the
//! iCloud Drive `FileIo` over the `mypass-platform` coordinated-file seam.
//!
//! Safe code only (`unsafe_code = "forbid"` through `[workspace.lints]`): every Objective-C call
//! stays in `mypass-platform`. The modules land in later stage 5 tasks; this is the scaffold.
