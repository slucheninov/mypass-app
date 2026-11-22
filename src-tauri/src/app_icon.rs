//! The configurable app icon. The running app shows the light or the dark variant of its key
//! (`icons/app-light.png`, `icons/app-dark.png`, made by `scripts/make-app-icons.mjs`) for the
//! chosen icon appearance: the Dock tile on macOS (`mypass_platform::AppIcon`), the window and taskbar
//! icons on Windows. Finder, Explorer, pinned shortcuts and installers keep the bundle icon, which
//! is the light variant.
//!
//! The decisions are here, as plain functions; `shell_runtime.rs` reads the inputs and applies the
//! result on the main thread.

use mypass_session::settings::AppIcon;
use std::sync::{Mutex, PoisonError};

/// The light variant (light tile, dark key), also the bundle icon.
pub const LIGHT_PNG: &[u8] = include_bytes!("../icons/app-light.png");
/// The dark variant (dark tile, light key).
pub const DARK_PNG: &[u8] = include_bytes!("../icons/app-dark.png");

/// Which of the two icons the app shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIconVariant {
    Light,
    Dark,
}

impl AppIconVariant {
    /// The encoded PNG of this variant.
    pub fn png(self) -> &'static [u8] {
        match self {
            Self::Light => LIGHT_PNG,
            Self::Dark => DARK_PNG,
        }
    }
}

/// The icon for the appearance setting: `Light` and `Dark` decide alone, `System` follows the OS
/// appearance (`system_dark`).
pub fn app_icon_variant(setting: AppIcon, system_dark: bool) -> AppIconVariant {
    match setting {
        AppIcon::Light => AppIconVariant::Light,
        AppIcon::Dark => AppIconVariant::Dark,
        AppIcon::System if system_dark => AppIconVariant::Dark,
        AppIcon::System => AppIconVariant::Light,
    }
}

/// The OS appearance from the best source there is: the platform's own reading (macOS, where the
/// app's forced appearance never hides it), else the theme a window reported, else light.
pub fn system_dark(platform: Option<bool>, window_dark: Option<bool>) -> bool {
    platform.or(window_dark).unwrap_or(false)
}

/// The variant last shown, so repeated theme events do not reset the icon each time.
#[derive(Debug, Default)]
pub struct AppIconState(Mutex<Option<AppIconVariant>>);

impl AppIconState {
    /// Whether `variant` must be applied: always when `force`d (a new window, or a framework that
    /// may have reset the icon), otherwise only when it differs from the last one. Records it.
    pub fn should_apply(&self, variant: AppIconVariant, force: bool) -> bool {
        let mut last = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let changed = *last != Some(variant);
        *last = Some(variant);
        changed || force
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_app_icons_decide_alone_and_system_follows_the_os() {
        for system_dark in [false, true] {
            assert_eq!(
                app_icon_variant(AppIcon::Light, system_dark),
                AppIconVariant::Light
            );
            assert_eq!(
                app_icon_variant(AppIcon::Dark, system_dark),
                AppIconVariant::Dark
            );
        }
        assert_eq!(
            app_icon_variant(AppIcon::System, true),
            AppIconVariant::Dark
        );
        assert_eq!(
            app_icon_variant(AppIcon::System, false),
            AppIconVariant::Light
        );
    }

    #[test]
    fn system_dark_prefers_the_platform_then_the_window_then_light() {
        assert!(system_dark(Some(true), Some(false)));
        assert!(!system_dark(Some(false), Some(true)));
        assert!(system_dark(None, Some(true)));
        assert!(!system_dark(None, Some(false)));
        assert!(!system_dark(None, None));
    }

    #[test]
    fn state_applies_changes_once_and_forced_always() {
        let state = AppIconState::default();
        assert!(state.should_apply(AppIconVariant::Light, false));
        assert!(!state.should_apply(AppIconVariant::Light, false));
        assert!(state.should_apply(AppIconVariant::Light, true));
        assert!(state.should_apply(AppIconVariant::Dark, false));
        assert!(!state.should_apply(AppIconVariant::Dark, false));
    }

    #[test]
    fn variants_embed_distinct_square_pngs() {
        const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
        for variant in [AppIconVariant::Light, AppIconVariant::Dark] {
            let png = variant.png();
            assert!(png.starts_with(SIGNATURE), "{variant:?}");
            let width = png
                .get(16..20)
                .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
            let height = png
                .get(20..24)
                .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
            assert_eq!(width, Some(512), "{variant:?}");
            assert_eq!(height, Some(512), "{variant:?}");
        }
        assert_ne!(LIGHT_PNG, DARK_PNG);
    }
}
