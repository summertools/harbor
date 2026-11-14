//! Release-asset classification and platform-aware selection.

use globset::{GlobBuilder, GlobMatcher};

use crate::error::{Error, Result};
use crate::github::{Asset, Release};
use crate::platform::Platform;

/// Supported release asset containers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssetKind {
    TarGz,
    Tar,
    Zip,
    Raw,
}

impl AssetKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::TarGz => "tar.gz",
            Self::Tar => "tar",
            Self::Zip => "zip",
            Self::Raw => "binary",
        }
    }

    #[must_use]
    pub const fn is_archive(self) -> bool {
        !matches!(self, Self::Raw)
    }

    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            Some(Self::TarGz)
        } else if lower.ends_with(".tar") {
            Some(Self::Tar)
        } else if lower.ends_with(".zip") {
            Some(Self::Zip)
        } else if lower.ends_with(".exe")
            || lower.ends_with(".appimage")
            || lower.ends_with(".bin")
            || !lower.contains('.')
        {
            Some(Self::Raw)
        } else {
            None
        }
    }
}

/// A release asset that could be installed on the current platform.
#[derive(Clone, Debug)]
pub struct AssetChoice {
    pub asset: Asset,
    pub kind: AssetKind,
    pub score: i32,
    /// True when the caller explicitly requested this asset with `--asset`.
    pub explicit: bool,
}

impl AssetChoice {
    #[must_use]
    pub fn size_label(&self) -> String {
        crate::format::bytes(self.asset.size)
    }

    /// True when the asset can run on the current platform. Explicitly
    /// requested assets are still installable even when they look unusual.
    #[must_use]
    pub fn compatible(&self) -> bool {
        self.explicit || self.score > 0
    }
}

/// Assets split by current-platform compatibility, sorted best-first.
/// Incompatible assets are retained so the UI can explain exactly why a
/// release cannot be installed instead of pretending the release itself is
/// missing.
#[derive(Clone, Debug, Default)]
pub struct AssetCatalog {
    pub compatible: Vec<AssetChoice>,
    pub incompatible: Vec<AssetChoice>,
}

impl AssetCatalog {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.compatible.is_empty() && self.incompatible.is_empty()
    }

    #[must_use]
    pub fn best(&self) -> Option<&AssetChoice> {
        self.compatible.first()
    }
}

/// Explain why an otherwise supported asset cannot run here.
#[must_use]
pub fn incompatibility(platform: Platform, name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    if platform.has_foreign_os(&lower) {
        return Some("different operating system");
    }
    if platform.has_foreign_arch(&lower) {
        return Some("different architecture");
    }
    if let Some(kind) = AssetKind::from_name(name) {
        if kind == AssetKind::Raw
            && lower.ends_with(".exe")
            && platform.os != crate::platform::Os::Windows
        {
            return Some("Windows executable");
        }
        if kind == AssetKind::Raw
            && lower.ends_with(".appimage")
            && platform.os != crate::platform::Os::Linux
        {
            return Some("Linux AppImage");
        }
    }
    None
}

/// Return every supported archive/binary, split into compatible and
/// incompatible groups. Metadata and unsupported containers are omitted.
#[must_use]
pub fn catalog_assets(platform: Platform, release: &Release) -> AssetCatalog {
    let mut catalog = AssetCatalog::default();
    for asset in &release.assets {
        if is_metadata_asset(&asset.name) {
            continue;
        }
        let Some(kind) = AssetKind::from_name(&asset.name) else {
            continue;
        };
        let score = score_asset(platform, &asset.name).unwrap_or(-1000);
        let choice = AssetChoice {
            asset: asset.clone(),
            kind,
            score,
            explicit: false,
        };
        if choice.compatible() {
            catalog.compatible.push(choice);
        } else {
            catalog.incompatible.push(choice);
        }
    }
    let sort = |choices: &mut Vec<AssetChoice>| {
        choices.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.asset.name.cmp(&right.asset.name))
        });
    };
    sort(&mut catalog.compatible);
    sort(&mut catalog.incompatible);
    catalog
}

/// True for checksums, signatures, SBOMs, installers, and source archives.
#[must_use]
pub fn is_metadata_asset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    const SUFFIXES: &[&str] = &[
        ".sha256",
        ".sha256sum",
        ".sha256.txt",
        ".sha512",
        ".sha512sum",
        ".sha1",
        ".md5",
        ".sig",
        ".asc",
        ".minisig",
        ".pem",
        ".crt",
        ".sbom",
        ".spdx",
        ".spdx.json",
        ".intoto",
        ".intoto.jsonl",
        ".json",
        ".txt",
        ".md",
        ".html",
        ".yml",
        ".yaml",
        ".deb",
        ".rpm",
        ".apk",
        ".msi",
        ".dmg",
        ".pkg",
        ".7z",
        ".tar.xz",
        ".txz",
        ".tar.zst",
        ".tzst",
    ];
    if SUFFIXES.iter().any(|suffix| lower.ends_with(suffix)) {
        return true;
    }
    if lower.contains("checksum") || lower.contains("sha256sum") {
        return true;
    }
    let first = crate::platform::split_tokens(&lower)
        .first()
        .copied()
        .unwrap_or("");
    if first == "source" || first == "sources" {
        return true;
    }
    lower.starts_with("readme")
        || lower.starts_with("license")
        || lower.starts_with("changelog")
        || lower.starts_with("notice")
}

/// Score one supported asset for the current platform. A strongly negative
/// score marks a foreign asset; [`select_assets`] filters those out while
/// [`catalog_assets`] keeps them for explanatory UI.
#[must_use]
pub fn score_asset(platform: Platform, name: &str) -> Option<i32> {
    if is_metadata_asset(name) {
        return None;
    }
    let kind = AssetKind::from_name(name)?;
    let lower = name.to_ascii_lowercase();
    let incompatible = incompatibility(platform, name).is_some();
    let tokens = crate::platform::split_tokens(&lower);
    let mut score = 0i32;
    if platform.names_os(&lower) {
        score += 40;
    } else {
        score += 1;
    }
    if platform.names_arch(&lower) {
        score += 30;
    } else if tokens.contains(&"universal") {
        score += 12;
    } else {
        score += 1;
    }
    if platform.os == crate::platform::Os::Linux {
        if lower.contains("musl") {
            score += 10;
        } else if lower.contains("gnu") {
            score += 5;
        }
    } else if platform.os == crate::platform::Os::Windows {
        if lower.contains("msvc") {
            score += 5;
        } else if lower.contains("gnu") {
            score += 2;
        }
    }
    score += match kind {
        AssetKind::TarGz => 10,
        AssetKind::Zip => 8,
        AssetKind::Tar => 6,
        AssetKind::Raw => 5,
    };
    if tokens.contains(&"debug") {
        score -= 30;
    }
    if tokens.contains(&"symbols") {
        score -= 40;
    }
    if incompatible {
        score -= 100;
    }
    Some(score)
}

/// Return all supported assets, best match first. `filter` is an optional
/// glob (for example `*linux-musl*` or `*aarch64*`).
pub fn select_assets(
    platform: Platform,
    release: &Release,
    filter: Option<&str>,
) -> Result<Vec<AssetChoice>> {
    let matcher = filter.map(compile_glob).transpose()?;
    let mut choices = Vec::new();
    for asset in &release.assets {
        if is_metadata_asset(&asset.name) {
            continue;
        }
        let Some(kind) = AssetKind::from_name(&asset.name) else {
            continue;
        };
        let explicit = matcher
            .as_ref()
            .is_some_and(|glob| glob.is_match(&asset.name));
        let score = score_asset(platform, &asset.name).unwrap_or(-1000);
        if matcher.is_some() {
            if !explicit {
                continue;
            }
        } else if score <= 0 {
            continue;
        }
        choices.push(AssetChoice {
            asset: asset.clone(),
            kind,
            score,
            explicit,
        });
    }
    if choices.is_empty() {
        let filter_note = filter.map_or_else(String::new, |filter| format!(" matching {filter:?}"));
        return Err(Error::NoAsset {
            platform: format!("{}{}", platform.label(), filter_note),
        });
    }
    choices.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.asset.name.cmp(&right.asset.name))
    });
    Ok(choices)
}

fn compile_glob(pattern: &str) -> Result<GlobMatcher> {
    GlobBuilder::new(pattern)
        .case_insensitive(true)
        .literal_separator(true)
        .build()
        .map(|glob| glob.compile_matcher())
        .map_err(|error| Error::msg(format!("invalid asset glob {pattern:?}: {error}")))
}

/// Pick the unambiguous best asset. Interactive callers should use the full
/// list so the user can resolve a tie themselves.
pub fn best_asset(choices: &[AssetChoice]) -> Result<&AssetChoice> {
    let Some(first) = choices.first() else {
        return Err(Error::NoAsset {
            platform: "current system".to_owned(),
        });
    };
    if first.explicit {
        return Ok(first);
    }
    if let Some(second) = choices.get(1)
        && second.score == first.score
    {
        let names = choices
            .iter()
            .take_while(|choice| choice.score == first.score)
            .map(|choice| choice.asset.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(Error::AmbiguousAsset(names));
    }
    Ok(first)
}

/// Find an explicitly named asset, used when the user picked one in the TUI.
pub fn asset_by_name<'a>(choices: &'a [AssetChoice], name: &str) -> Option<&'a AssetChoice> {
    choices.iter().find(|choice| choice.asset.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::Asset;
    use crate::platform::{Arch, Libc, Os};

    fn release(names: &[&str]) -> Release {
        Release {
            tag_name: "v1.0.0".to_owned(),
            name: None,
            body: None,
            draft: false,
            prerelease: false,
            published_at: None,
            assets: names
                .iter()
                .enumerate()
                .map(|(index, name)| Asset {
                    id: index as u64,
                    name: (*name).to_owned(),
                    size: 1024,
                    download_count: None,
                    content_type: None,
                    browser_download_url: format!(
                        "https://github.com/o/r/releases/download/v1/{name}"
                    ),
                })
                .collect(),
        }
    }

    fn linux_x64() -> Platform {
        Platform {
            os: Os::Linux,
            arch: Arch::X86_64,
            libc: Libc::Gnu,
        }
    }

    #[test]
    fn picks_musl_archive_and_ignores_checksums() {
        let release = release(&[
            "rtop-v1-x86_64-unknown-linux-musl.tar.gz",
            "rtop-v1-x86_64-unknown-linux-musl.tar.gz.sha256",
            "rtop-v1-aarch64-unknown-linux-musl.tar.gz",
            "source.tar.gz",
            "README.md",
        ]);
        let choices = select_assets(linux_x64(), &release, None).unwrap();
        assert_eq!(
            choices[0].asset.name,
            "rtop-v1-x86_64-unknown-linux-musl.tar.gz"
        );
        assert_eq!(choices.len(), 1);
    }

    #[test]
    fn catalog_explains_rtop_x86_64_assets_on_arm64_linux() {
        let release = release(&[
            "rtop-linux-x86_64.tar.gz",
            "rtop-macos-aarch64.tar.gz",
            "rtop-macos-x86_64.tar.gz",
            "rtop-windows-x86_64.zip",
            "SHA256SUMS",
        ]);
        let arm = Platform {
            os: Os::Linux,
            arch: Arch::Aarch64,
            libc: Libc::Gnu,
        };
        let catalog = catalog_assets(arm, &release);
        assert!(catalog.compatible.is_empty());
        assert_eq!(catalog.incompatible.len(), 4);
        let reasons: Vec<&str> = catalog
            .incompatible
            .iter()
            .filter_map(|choice| incompatibility(arm, &choice.asset.name))
            .collect();
        assert!(reasons.contains(&"different architecture"));
        assert!(reasons.contains(&"different operating system"));
        assert!(matches!(
            select_assets(arm, &release, None),
            Err(Error::NoAsset { .. })
        ));
    }

    #[test]
    fn platform_mismatches_are_rejected() {
        let release = release(&[
            "tool-aarch64-apple-darwin.tar.gz",
            "tool-x86_64-pc-windows-msvc.zip",
            "tool-x86_64-unknown-linux-gnu.tar.gz",
        ]);
        let choices = select_assets(linux_x64(), &release, None).unwrap();
        assert_eq!(choices.len(), 1);
        assert_eq!(
            choices[0].asset.name,
            "tool-x86_64-unknown-linux-gnu.tar.gz"
        );
    }

    #[test]
    fn explicit_glob_can_pick_a_special_asset() {
        let release = release(&[
            "tool-x86_64-unknown-linux-musl.tar.gz",
            "tool-x86_64-unknown-linux-gnu.zip",
        ]);
        let choices = select_assets(linux_x64(), &release, Some("*gnu*")).unwrap();
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].kind, AssetKind::Zip);
        assert!(choices[0].explicit);
    }

    #[test]
    fn macos_and_windows_assets_are_selected_by_platform() {
        let release = release(&[
            "tool-aarch64-apple-darwin.tar.gz",
            "tool-x86_64-apple-darwin.tar.gz",
            "tool-x86_64-pc-windows-msvc.zip",
            "tool-x86_64-unknown-linux-musl.tar.gz",
        ]);
        let mac = Platform {
            os: Os::Macos,
            arch: Arch::Aarch64,
            libc: Libc::Unknown,
        };
        let choices = select_assets(mac, &release, None).unwrap();
        assert_eq!(choices[0].asset.name, "tool-aarch64-apple-darwin.tar.gz");

        let windows = Platform {
            os: Os::Windows,
            arch: Arch::X86_64,
            libc: Libc::Msvc,
        };
        let choices = select_assets(windows, &release, None).unwrap();
        assert_eq!(choices[0].asset.name, "tool-x86_64-pc-windows-msvc.zip");
    }

    #[test]
    fn windows_executables_are_not_selected_on_linux() {
        let release = release(&["tool.exe", "tool-linux-x86_64.tar.gz"]);
        let choices = select_assets(linux_x64(), &release, None).unwrap();
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].asset.name, "tool-linux-x86_64.tar.gz");
    }

    #[test]
    fn identical_scores_are_ambiguous() {
        let release = release(&["tool-linux-x86_64.tar.gz", "other-linux-x86_64.tar.gz"]);
        let choices = select_assets(linux_x64(), &release, None).unwrap();
        assert!(matches!(
            best_asset(&choices),
            Err(Error::AmbiguousAsset(_))
        ));
    }
}
