//! Conservative automatic-update decisions.
//!
//! Auto-update is deliberately stricter than a manual install:
//!
//! * the repository must already be managed by Harbor;
//! * the installed executable must still exist;
//! * the installed tag and latest tag must both be SemVer;
//! * the latest tag must be strictly newer (never downgrade);
//! * a compatible asset and a published SHA-256 checksum are required;
//! * unmanaged files, missing files, and the currently running Harbor binary
//!   are skipped.
//!
//! Callers must also install with `allow_unverified` disabled. The helpers in
//! this module never enable unverified updates.

use std::fs;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::github::{Release, RepoRef};
use crate::install::{self, InstallPlan, InstallRequest, InstalledPackage};
use crate::platform::{self, BinDir, Platform};
use crate::release;
use crate::version;

/// A prepared, checksum-backed update for one managed executable.
#[derive(Clone, Debug)]
pub struct AutoUpdatePlan {
    pub package: InstalledPackage,
    pub plan: InstallPlan,
}

impl AutoUpdatePlan {
    #[must_use]
    pub fn from_version(&self) -> &str {
        &self.package.version
    }

    #[must_use]
    pub fn to_version(&self) -> &str {
        &self.plan.release.tag_name
    }
}

/// Result of evaluating one installed package against its latest release.
#[derive(Clone, Debug)]
pub enum Decision {
    /// A safe update is ready.
    Update(Box<AutoUpdatePlan>),
    /// Already current, or the latest release is not newer.
    Current,
    /// An update exists but Harbor refuses to apply it automatically.
    Skip { reason: String },
}

/// Force checksum verification for an auto-update install. This overrides a
/// permissive `allow_unverified = true` from the general config.
#[must_use]
pub fn checksum_only_config(mut config: Config) -> Config {
    config.allow_unverified = false;
    config
}

/// Decide whether `release` is a safe automatic update for `package`.
pub fn decide(
    config: &Config,
    platform: Platform,
    package: &InstalledPackage,
    release: &Release,
) -> Decision {
    if release.draft {
        return Decision::Skip {
            reason: "refusing to install a draft release".to_owned(),
        };
    }
    if release.prerelease && !config.include_prereleases {
        return Decision::Skip {
            reason: "refusing to auto-install a prerelease without include_prereleases".to_owned(),
        };
    }
    if !package.present {
        return Decision::Skip {
            reason: "the recorded executable is missing".to_owned(),
        };
    }
    if is_running_executable(&package.path) {
        return Decision::Skip {
            reason: "refusing to update the running harbor executable".to_owned(),
        };
    }
    match version::is_newer(&package.version, &release.tag_name) {
        Some(true) => {}
        Some(false) => return Decision::Current,
        None => {
            return Decision::Skip {
                reason: format!(
                    "cannot compare non-SemVer tags {:?} and {:?}",
                    package.version, release.tag_name
                ),
            };
        }
    }

    let catalog = release::catalog_assets(platform, release);
    let Some(choice) = catalog.best() else {
        return Decision::Skip {
            reason: format!(
                "latest release has no compatible asset for {}",
                platform.label()
            ),
        };
    };
    if crate::checksum::checksum_assets(release, &choice.asset).is_empty() {
        return Decision::Skip {
            reason: "latest release has no published SHA-256 checksum".to_owned(),
        };
    }
    if !package
        .path
        .parent()
        .is_some_and(|parent| same_directory(parent, &package.bin_dir))
    {
        return Decision::Skip {
            reason: "recorded executable is outside its install directory".to_owned(),
        };
    }
    let Some(target_name) = package
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
    else {
        return Decision::Skip {
            reason: "recorded executable has no valid filename".to_owned(),
        };
    };
    let Ok(repo) = RepoRef::parse(&package.repo) else {
        return Decision::Skip {
            reason: "recorded repository name is invalid".to_owned(),
        };
    };
    let bin_dir = BinDir {
        path: package.bin_dir.clone(),
        in_path: platform::path_contains(&package.bin_dir),
    };
    let request = InstallRequest {
        repo,
        tag: Some(release.tag_name.clone()),
        asset_name: Some(choice.asset.name.clone()),
        asset_glob: None,
        binary: None,
        name: Some(target_name),
        force: false,
        allow_unverified: false,
        expected_current: Some(package.version.clone()),
    };
    match install::build_plan(
        config,
        platform,
        &bin_dir,
        request,
        release.clone(),
        choice.clone(),
    ) {
        Ok(plan) if plan.checksum_available() => Decision::Update(Box::new(AutoUpdatePlan {
            package: package.clone(),
            plan,
        })),
        Ok(_) => Decision::Skip {
            reason: "prepared update lost its checksum source".to_owned(),
        },
        Err(error) => Decision::Skip {
            reason: format!("cannot prepare update: {error}"),
        },
    }
}

/// Run one prepared auto-update. The caller must pass a checksum-only config.
pub fn install(
    plan: &AutoUpdatePlan,
    installer: &install::Installer<'_>,
    mut progress: impl FnMut(crate::install::ProgressEvent),
) -> Result<crate::install::InstallOutcome> {
    if !plan.plan.checksum_available() {
        return Err(Error::ChecksumMissing(plan.plan.choice.asset.name.clone()));
    }
    if plan.plan.request.force {
        return Err(Error::msg(
            "refusing an automatic update that requests forced replacement",
        ));
    }
    if plan.plan.request.expected_current.as_deref() != Some(plan.from_version()) {
        return Err(Error::msg(
            "refusing an automatic update without its ledger baseline",
        ));
    }
    if version::is_newer(plan.from_version(), plan.to_version()) != Some(true) {
        return Err(Error::msg(format!(
            "refusing a non-forward automatic update from {} to {}",
            plan.from_version(),
            plan.to_version()
        )));
    }
    installer.install(&plan.plan, &mut progress)
}

/// Ledger keys are stored lowercased on Windows, while `bin_dir` keeps the
/// canonical on-disk case. Compare with the platform's usual semantics.
fn same_directory(left: &std::path::Path, right: &std::path::Path) -> bool {
    if cfg!(windows) {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}

fn is_running_executable(path: &std::path::Path) -> bool {
    let Ok(current) = std::env::current_exe().and_then(fs::canonicalize) else {
        return false;
    };
    fs::canonicalize(path).is_ok_and(|candidate| candidate == current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::Asset;

    fn release(tag: &str, names: &[&str]) -> Release {
        Release {
            tag_name: tag.to_owned(),
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
                    size: 10,
                    download_count: None,
                    content_type: None,
                    browser_download_url: format!(
                        "https://github.com/owner/tool/releases/download/{tag}/{name}"
                    ),
                })
                .collect(),
        }
    }

    fn package(version: &str) -> InstalledPackage {
        InstalledPackage {
            repo: "owner/tool".to_owned(),
            version: version.to_owned(),
            path: std::path::PathBuf::from("/tmp/harbor-auto-test/tool"),
            bin_dir: std::path::PathBuf::from("/tmp/harbor-auto-test"),
            installed_at: 1,
            binaries: 1,
            present: true,
        }
    }

    fn config() -> Config {
        Config::default()
    }

    #[test]
    fn newer_checksummed_compatible_release_updates() {
        let release = release(
            "v2.0.0",
            &["tool-v2.0.0.tar.gz", "tool-v2.0.0.tar.gz.sha256"],
        );
        let decision = decide(&config(), Platform::current(), &package("v1.0.0"), &release);
        assert!(matches!(decision, Decision::Update(_)), "{decision:?}");
    }

    #[test]
    fn same_or_older_release_is_current() {
        for tag in ["v1.0.0", "v0.9.9"] {
            let release = release(tag, &["tool.tar.gz", "tool.tar.gz.sha256"]);
            let decision = decide(&config(), Platform::current(), &package("v1.0.0"), &release);
            assert!(matches!(decision, Decision::Current), "{tag}: {decision:?}");
        }
    }

    #[test]
    fn draft_and_unexpected_prerelease_releases_are_skipped() {
        let mut draft = release("v2.0.0", &["tool.tar.gz", "tool.tar.gz.sha256"]);
        draft.draft = true;
        let decision = decide(&config(), Platform::current(), &package("v1.0.0"), &draft);
        assert!(matches!(decision, Decision::Skip { .. }));

        let mut prerelease = release("v2.0.0-rc.1", &["tool.tar.gz", "tool.tar.gz.sha256"]);
        prerelease.prerelease = true;
        let decision = decide(
            &config(),
            Platform::current(),
            &package("v1.0.0"),
            &prerelease,
        );
        assert!(matches!(decision, Decision::Skip { .. }));
    }

    #[test]
    fn missing_checksum_is_skipped() {
        let release = release("v2.0.0", &["tool.tar.gz"]);
        let decision = decide(&config(), Platform::current(), &package("v1.0.0"), &release);
        assert!(matches!(decision, Decision::Skip { .. }));
    }

    #[test]
    fn non_semver_tags_are_never_auto_updated() {
        let release = release("release-2", &["tool.tar.gz", "tool.tar.gz.sha256"]);
        let decision = decide(
            &config(),
            Platform::current(),
            &package("release-1"),
            &release,
        );
        assert!(matches!(decision, Decision::Skip { .. }));
    }

    #[test]
    fn directory_comparison_follows_platform_semantics() {
        let left = std::path::Path::new("/tmp/harbor-auto-test");
        assert!(same_directory(left, left));
        #[cfg(windows)]
        assert!(same_directory(
            std::path::Path::new("C:\\Tools\\Harbor"),
            std::path::Path::new("c:\\tools\\harbor")
        ));
        #[cfg(not(windows))]
        assert!(!same_directory(
            std::path::Path::new("/tmp/Harbor"),
            std::path::Path::new("/tmp/harbor")
        ));
    }

    #[test]
    fn executable_outside_the_recorded_install_directory_is_skipped() {
        let mut package = package("v1.0.0");
        package.bin_dir = std::path::PathBuf::from("/tmp/harbor-auto-test/elsewhere");
        let release = release("v2.0.0", &["tool.tar.gz", "tool.tar.gz.sha256"]);
        let decision = decide(&config(), Platform::current(), &package, &release);
        assert!(matches!(decision, Decision::Skip { .. }));
    }

    #[test]
    fn missing_executable_is_skipped() {
        let mut package = package("v1.0.0");
        package.present = false;
        let release = release("v2.0.0", &["tool.tar.gz", "tool.tar.gz.sha256"]);
        let decision = decide(&config(), Platform::current(), &package, &release);
        assert!(matches!(decision, Decision::Skip { .. }));
    }
}
