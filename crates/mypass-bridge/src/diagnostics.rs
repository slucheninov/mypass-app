//! The latest browser diagnostics report per paired browser.
//!
//! In memory on purpose: a restart of the app means there is no current observation, and the UI
//! then shows "not reported yet" rather than a stale answer.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use uuid::Uuid;

use crate::protocol::BrowserDiagnosticsReport;

/// A report and when it arrived.
#[derive(Debug, Clone, PartialEq)]
pub struct DiagnosticsSnapshot {
    pub report: BrowserDiagnosticsReport,
    /// Unix seconds.
    pub received_at: i64,
}

/// Transient store keyed by paired-browser id.
#[derive(Debug, Default)]
pub struct DiagnosticsStore {
    items: Mutex<HashMap<Uuid, DiagnosticsSnapshot>>,
}

impl DiagnosticsStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn items(&self) -> MutexGuard<'_, HashMap<Uuid, DiagnosticsSnapshot>> {
        self.items.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Keeps `report` as the latest one of `browser_id`. An invalid report
    /// (`BrowserDiagnosticsReport::is_valid`) is ignored, the previous snapshot stays, and `false`
    /// is returned.
    pub fn record(
        &self,
        report: BrowserDiagnosticsReport,
        browser_id: Uuid,
        received_at: i64,
    ) -> bool {
        if !report.is_valid() {
            return false;
        }
        self.items().insert(
            browser_id,
            DiagnosticsSnapshot {
                report,
                received_at,
            },
        );
        true
    }

    #[must_use]
    pub fn get(&self, id: Uuid) -> Option<DiagnosticsSnapshot> {
        self.items().get(&id).cloned()
    }

    pub fn remove(&self, id: Uuid) {
        self.items().remove(&id);
    }

    #[must_use]
    pub fn all(&self) -> Vec<(Uuid, DiagnosticsSnapshot)> {
        self.items()
            .iter()
            .map(|(id, snap)| (*id, snap.clone()))
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::protocol::{ExtensionScanState, PermissionState, ProxyState};

    fn sample() -> BrowserDiagnosticsReport {
        BrowserDiagnosticsReport {
            proxy_state: ProxyState::Active,
            permission_state: PermissionState::Granted,
            password_saving_enabled: Some(true),
            extension_scan_state: ExtensionScanState::Complete,
            enabled_managers: vec!["Other Manager".into()],
        }
    }

    #[test]
    fn report_bounds_and_consistent_unknown_states() {
        assert!(sample().is_valid());

        let mut r = sample();
        r.enabled_managers = (0..21).map(|i| format!("M{i}")).collect();
        assert!(!r.is_valid(), "21 managers");
        r.enabled_managers.truncate(20);
        assert!(r.is_valid(), "20 managers");

        let mut r = sample();
        r.enabled_managers = vec!["x".repeat(121)];
        assert!(!r.is_valid(), "121-char name");
        r.enabled_managers = vec!["x".repeat(120)];
        assert!(r.is_valid(), "120-char name");

        let mut r = sample();
        r.enabled_managers = vec!["Manager\nInjected".into()];
        assert!(!r.is_valid(), "control character");

        let mut r = sample();
        r.enabled_managers = vec![" ".into()];
        assert!(!r.is_valid(), "blank name");

        let mut r = sample();
        r.extension_scan_state = ExtensionScanState::Unavailable;
        assert!(!r.is_valid(), "unavailable scan with a manager");

        let mut r = sample();
        r.permission_state = PermissionState::NotGranted;
        r.extension_scan_state = ExtensionScanState::NotChecked;
        r.enabled_managers = vec![];
        r.password_saving_enabled = Some(true);
        assert!(!r.is_valid(), "not granted but a saving answer");
        r.password_saving_enabled = None;
        assert!(r.is_valid(), "not granted, nothing claimed");

        let mut r = sample();
        r.permission_state = PermissionState::Unavailable;
        r.extension_scan_state = ExtensionScanState::NotChecked;
        r.enabled_managers = vec![];
        r.password_saving_enabled = None;
        assert!(r.is_valid(), "unavailable, nothing claimed");
    }

    #[test]
    fn record_then_get_returns_snapshot() {
        let store = DiagnosticsStore::new();
        let id = Uuid::new_v4();
        assert_eq!(store.get(id), None);
        store.record(sample(), id, 42);
        let snap = store.get(id).unwrap();
        assert_eq!(snap.received_at, 42);
        assert_eq!(snap.report, sample());
        store.record(sample(), id, 43);
        assert_eq!(store.get(id).unwrap().received_at, 43);
        assert_eq!(store.all().len(), 1);
    }

    #[test]
    fn record_ignores_invalid_reports_and_keeps_the_previous_snapshot() {
        let store = DiagnosticsStore::new();
        let id = Uuid::new_v4();
        let mut bad = sample();
        bad.enabled_managers = (0..21).map(|i| format!("M{i}")).collect();
        assert!(!store.record(bad.clone(), id, 1));
        assert_eq!(store.get(id), None);
        assert!(store.record(sample(), id, 2));
        assert!(!store.record(bad, id, 3));
        assert_eq!(store.get(id).unwrap().received_at, 2);
    }

    #[test]
    fn format_characters_in_manager_names_are_rejected() {
        for name in ["Man\u{202E}ager", "Man\u{200B}ager", "\u{FEFF}Manager"] {
            let mut r = sample();
            r.enabled_managers = vec![name.into()];
            assert!(!r.is_valid(), "{name:?}");
        }
    }

    #[test]
    fn remove_drops_snapshot() {
        let store = DiagnosticsStore::new();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        store.record(sample(), a, 1);
        store.record(sample(), b, 2);
        store.remove(a);
        assert_eq!(store.get(a), None);
        assert!(store.get(b).is_some());
        assert_eq!(store.all().len(), 1);
        store.remove(a);
    }
}
