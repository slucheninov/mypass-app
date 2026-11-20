//! Release artifact identity only. Update checks never retain binary download URLs.
use crate::SemanticVersion;
use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleasePlatform {
    MacosArm64,
    WindowsX64,
}

/// Detect the build target, including architecture; other targets have no published artifact.
pub fn current_release_platform() -> Option<ReleasePlatform> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some(ReleasePlatform::MacosArm64)
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some(ReleasePlatform::WindowsX64)
    } else {
        None
    }
}

pub fn select_asset<'a>(
    assets: &'a [ReleaseAsset],
    platform: ReleasePlatform,
    version: &SemanticVersion,
) -> Option<&'a ReleaseAsset> {
    let suffix = match platform {
        ReleasePlatform::MacosArm64 => "macos-arm64.dmg",
        ReleasePlatform::WindowsX64 => "windows-x64.zip",
    };
    let name = format!("MyPass-{version}-{suffix}");
    assets.iter().find(|asset| asset.name == name)
}

/// macOS uses the shared manifest; Windows uses the ZIP's exact `.sha256` sibling.
/// Presence gates the banner only: no checksum or payload is downloaded by this crate.
pub fn checksum_asset_for<'a>(
    asset: &ReleaseAsset,
    assets: &'a [ReleaseAsset],
) -> Option<&'a ReleaseAsset> {
    let rest = asset.name.strip_prefix("MyPass-")?;
    let (version, platform) = match rest.strip_suffix("-macos-arm64.dmg") {
        Some(version) => (version, ReleasePlatform::MacosArm64),
        None => (
            rest.strip_suffix("-windows-x64.zip")?,
            ReleasePlatform::WindowsX64,
        ),
    };
    let version = SemanticVersion::parse(version).ok()?;
    // Reuse exact selection so lookalike/noncanonical names cannot acquire a checksum.
    select_asset(std::slice::from_ref(asset), platform, &version)?;
    let name = match platform {
        ReleasePlatform::MacosArm64 => "SHA256SUMS.txt".into(),
        ReleasePlatform::WindowsX64 => format!("{}.sha256", asset.name),
    };
    assets.iter().find(|candidate| candidate.name == name)
}
