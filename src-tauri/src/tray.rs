//! The tray icon: its menu model (pure, tested), the ids that tie a menu click to an action (pure,
//! tested) and the thin Tauri glue that builds the icon, the menu and the click-to-toggle panel.
//!
//! The icon and its menu are built only from Rust (no webview permission); a menu click or an
//! icon click is a Rust callback, never an event a webview could forge.

use crate::i18n;
use mypass_session::settings::Language;
use mypass_session::SessionState;

/// The id of the one tray icon.
pub const TRAY_ID: &str = "main";
/// The label of the tray panel window.
pub const PANEL_WINDOW: &str = "tray-panel";
/// At most this many recent entries are listed while unlocked.
pub const MAX_RECENTS: usize = 5;

/// A recently used entry as the tray needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentEntry {
    pub id: String,
    pub title: String,
}

/// One row of the tray menu (separators are the glue's business).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayItem {
    Recent {
        id: String,
        title: String,
    },
    Unlock,
    Lock,
    OpenApp,
    Settings,
    /// A newer version is available (stage 5 supplies it).
    UpdateAvailable(String),
    Quit,
}

impl TrayItem {
    /// The row's text in `language` (a recent entry shows its own title).
    pub fn label(&self, language: Language) -> String {
        match self {
            Self::Recent { title, .. } => title.clone(),
            Self::Unlock => i18n::t(language, "Unlock", &[]),
            Self::Lock => i18n::t(language, "Lock", &[]),
            Self::OpenApp => i18n::t(language, "Open MyPass", &[]),
            Self::Settings => i18n::t(language, "Settings\u{2026}", &[]),
            Self::UpdateAvailable(version) => {
                i18n::t(language, "MyPass %@ available\u{2026}", &[version])
            }
            Self::Quit => i18n::t(language, "Quit", &[]),
        }
    }

    /// The stable menu id of the row.
    pub fn id(&self) -> String {
        match self {
            Self::Recent { id, .. } => format!("recent:{id}"),
            Self::Unlock => "unlock".to_owned(),
            Self::Lock => "lock".to_owned(),
            Self::OpenApp => "open".to_owned(),
            Self::Settings => "settings".to_owned(),
            Self::UpdateAvailable(_) => "update".to_owned(),
            Self::Quit => "quit".to_owned(),
        }
    }
}

/// The tray menu as data: the rows in order and the language their labels are in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayMenuModel {
    pub items: Vec<TrayItem>,
    pub language: Language,
}

impl TrayMenuModel {
    /// The labels of the rows, in order.
    pub fn labels(&self) -> Vec<String> {
        self.items
            .iter()
            .map(|item| item.label(self.language))
            .collect()
    }
}

/// Builds the menu: while unlocked up to five recent entries and `Lock`; while locked `Unlock`
/// (nothing to unlock without a vault); then `Open MyPass`, `Settings…`, the update row when
/// `update` is `Some`, and `Quit`.
pub fn tray_menu_model(
    status: SessionState,
    recents: &[RecentEntry],
    language: Language,
    update: Option<&str>,
) -> TrayMenuModel {
    let mut items = Vec::new();
    match status {
        SessionState::Unlocked => {
            items.extend(
                recents
                    .iter()
                    .take(MAX_RECENTS)
                    .map(|entry| TrayItem::Recent {
                        id: entry.id.clone(),
                        title: entry.title.clone(),
                    }),
            );
            items.push(TrayItem::Lock);
        }
        SessionState::Locked => items.push(TrayItem::Unlock),
        SessionState::NoVault => {}
    }
    items.push(TrayItem::OpenApp);
    items.push(TrayItem::Settings);
    if let Some(version) = update {
        items.push(TrayItem::UpdateAvailable(version.to_owned()));
    }
    items.push(TrayItem::Quit);
    TrayMenuModel { items, language }
}

/// Whether a separator goes between two consecutive rows: after the recent entries, before
/// `Open MyPass` and before `Quit`.
pub fn separator_before(previous: &TrayItem, item: &TrayItem) -> bool {
    matches!(previous, TrayItem::Recent { .. }) && !matches!(item, TrayItem::Recent { .. })
        || matches!(item, TrayItem::OpenApp | TrayItem::Quit)
}

/// What a menu click does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    /// Show and focus the main window (`Unlock`, `Open MyPass`, the update row).
    ShowMain,
    /// Open an entry in the main window.
    OpenEntry(String),
    Lock,
    OpenSettings,
    Quit,
}

/// The action of a menu id (`None` for an id this menu never produced).
pub fn action_for_id(id: &str) -> Option<TrayAction> {
    if let Some(entry) = id.strip_prefix("recent:") {
        return Some(TrayAction::OpenEntry(entry.to_owned()));
    }
    match id {
        "unlock" | "open" | "update" => Some(TrayAction::ShowMain),
        "lock" => Some(TrayAction::Lock),
        "settings" => Some(TrayAction::OpenSettings),
        "quit" => Some(TrayAction::Quit),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recents(n: usize) -> Vec<RecentEntry> {
        (0..n)
            .map(|i| RecentEntry {
                id: format!("{i:032x}"),
                title: format!("Entry {i}"),
            })
            .collect()
    }

    #[test]
    fn tray_menu_model_locked_has_unlock_and_open_and_quit_only() {
        let model = tray_menu_model(SessionState::Locked, &recents(3), Language::En, None);
        assert_eq!(
            model.items,
            [
                TrayItem::Unlock,
                TrayItem::OpenApp,
                TrayItem::Settings,
                TrayItem::Quit
            ]
        );
        assert_eq!(
            model.labels(),
            ["Unlock", "Open MyPass", "Settings\u{2026}", "Quit"]
        );
    }

    #[test]
    fn tray_menu_model_unlocked_lists_at_most_five_recents_then_lock() {
        let given = recents(7);
        let model = tray_menu_model(SessionState::Unlocked, &given, Language::En, None);
        let titles: Vec<_> = model.items.iter().take(5).collect();
        for (item, entry) in titles.iter().zip(&given) {
            assert_eq!(
                **item,
                TrayItem::Recent {
                    id: entry.id.clone(),
                    title: entry.title.clone()
                }
            );
        }
        assert_eq!(model.items.get(5), Some(&TrayItem::Lock));
        assert_eq!(
            model.items.len(),
            5 + 4,
            "recents, Lock, Open, Settings, Quit"
        );
        assert!(!model.items.contains(&TrayItem::Unlock));
    }

    #[test]
    fn tray_menu_without_a_vault_offers_neither_lock_nor_unlock() {
        let model = tray_menu_model(SessionState::NoVault, &recents(2), Language::En, None);
        assert_eq!(
            model.items,
            [TrayItem::OpenApp, TrayItem::Settings, TrayItem::Quit]
        );
    }

    #[test]
    fn tray_menu_update_row_follows_settings_and_is_absent_without_update() {
        for status in [SessionState::Locked, SessionState::Unlocked] {
            let with = tray_menu_model(status, &[], Language::En, Some("0.7.0"));
            let at = with
                .items
                .iter()
                .position(|i| *i == TrayItem::Settings)
                .unwrap();
            assert_eq!(
                with.items.get(at + 1),
                Some(&TrayItem::UpdateAvailable("0.7.0".into()))
            );
            assert_eq!(with.items.last(), Some(&TrayItem::Quit));
            assert_eq!(
                with.items.get(at + 1).unwrap().label(Language::En),
                "MyPass 0.7.0 available\u{2026}"
            );
            let without = tray_menu_model(status, &[], Language::En, None);
            assert!(!without
                .items
                .iter()
                .any(|i| matches!(i, TrayItem::UpdateAvailable(_))));
        }
    }

    #[test]
    fn tray_menu_labels_follow_language() {
        let uk_lock = i18n::lookup(i18n::Language::Uk, "Lock").unwrap();
        let model = tray_menu_model(SessionState::Unlocked, &[], Language::Uk, None);
        assert_eq!(model.labels().first().map(String::as_str), Some(uk_lock));
        assert_ne!(uk_lock, "Lock");
        // An explicit language never reads the system locale.
        let ru = tray_menu_model(SessionState::Locked, &[], Language::Ru, None);
        assert_eq!(
            ru.labels().first().map(String::as_str),
            i18n::lookup(i18n::Language::Ru, "Unlock")
        );
    }

    #[test]
    fn separators_group_the_menu() {
        let model = tray_menu_model(SessionState::Unlocked, &recents(2), Language::En, Some("1"));
        let mut layout = Vec::new();
        let mut previous: Option<&TrayItem> = None;
        for item in &model.items {
            if previous.is_some_and(|p| separator_before(p, item)) {
                layout.push("-");
            }
            layout.push(match item {
                TrayItem::Recent { .. } => "recent",
                TrayItem::Lock => "lock",
                TrayItem::OpenApp => "open",
                TrayItem::Settings => "settings",
                TrayItem::UpdateAvailable(_) => "update",
                TrayItem::Quit => "quit",
                TrayItem::Unlock => "unlock",
            });
            previous = Some(item);
        }
        assert_eq!(
            layout,
            ["recent", "recent", "-", "lock", "-", "open", "settings", "update", "-", "quit"]
        );
    }

    #[test]
    fn menu_ids_round_trip_to_actions() {
        let entry = "0123456789abcdef0123456789abcdef";
        let item = TrayItem::Recent {
            id: entry.into(),
            title: "T".into(),
        };
        assert_eq!(
            action_for_id(&item.id()),
            Some(TrayAction::OpenEntry(entry.into()))
        );
        assert_eq!(
            action_for_id(&TrayItem::Unlock.id()),
            Some(TrayAction::ShowMain)
        );
        assert_eq!(
            action_for_id(&TrayItem::OpenApp.id()),
            Some(TrayAction::ShowMain)
        );
        assert_eq!(
            action_for_id(&TrayItem::UpdateAvailable("1".into()).id()),
            Some(TrayAction::ShowMain)
        );
        assert_eq!(action_for_id(&TrayItem::Lock.id()), Some(TrayAction::Lock));
        assert_eq!(
            action_for_id(&TrayItem::Settings.id()),
            Some(TrayAction::OpenSettings)
        );
        assert_eq!(action_for_id(&TrayItem::Quit.id()), Some(TrayAction::Quit));
        assert_eq!(action_for_id("nonsense"), None);
    }
}
