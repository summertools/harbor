//! Release installation: checksum verification, safe extraction, atomic
//! placement, install ledger, and overwrite protection.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::archive::{self, BinaryCandidate, ExtractLimits};
use crate::checksum;
use crate::config::Config;
use crate::download;
use crate::error::{Error, Result};
use crate::github::{GitHubClient, Release, RepoRef};
use crate::platform::{self, BinDir, Platform};
use crate::release::{self, AssetChoice};

/// How far an install has progressed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InstallStage {
    #[default]
    Checksum,
    Downloading,
    Verifying,
    Extracting,
    Installing,
    Complete,
}

impl InstallStage {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Checksum => "fetching checksum",
            Self::Downloading => "downloading release asset",
            Self::Verifying => "verifying SHA-256",
            Self::Extracting => "extracting archive",
            Self::Installing => "installing executable",
            Self::Complete => "complete",
        }
    }
}

/// Throttled progress event sent to the CLI/TUI.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProgressEvent {
    pub stage: InstallStage,
    pub current: u64,
    pub total: Option<u64>,
}

/// User-selected install inputs.
#[derive(Clone, Debug)]
pub struct InstallRequest {
    pub repo: RepoRef,
    pub tag: Option<String>,
    /// Exact asset name selected in the TUI.
    pub asset_name: Option<String>,
    /// Optional glob used by non-interactive `--asset`.
    pub asset_glob: Option<String>,
    /// Optional filename (or relative path) inside the archive.
    pub binary: Option<String>,
    /// Optional destination binary name.
    pub name: Option<String>,
    pub force: bool,
    pub allow_unverified: bool,
}

/// A fully resolved installation that can be shown for confirmation or used
/// directly by the non-interactive installer.
#[derive(Clone, Debug)]
pub struct InstallPlan {
    pub request: InstallRequest,
    pub release: Release,
    pub choice: AssetChoice,
    pub checksum_assets: Vec<String>,
    pub bin_dir: PathBuf,
    pub in_path: bool,
    pub target_hint: String,
    pub warning: Option<String>,
}

impl InstallPlan {
    #[must_use]
    pub fn checksum_available(&self) -> bool {
        !self.checksum_assets.is_empty()
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.release.tag_name
    }
}

/// Result of a successful install.
#[derive(Clone, Debug)]
pub struct InstallOutcome {
    pub path: PathBuf,
    pub repo: String,
    pub version: String,
    pub asset: String,
    pub sha256: String,
    pub bytes: u64,
    pub checksum_verified: bool,
    pub already_current: bool,
    pub warning: Option<String>,
}

/// Installer bound to one config, platform, and executable directory.
pub struct Installer<'a> {
    client: &'a GitHubClient,
    config: &'a Config,
    platform: Platform,
    bin_dir: BinDir,
}

impl<'a> Installer<'a> {
    #[must_use]
    pub fn new(client: &'a GitHubClient, config: &'a Config, bin_dir: BinDir) -> Self {
        Self {
            client,
            config,
            platform: Platform::current(),
            bin_dir,
        }
    }

    /// Resolve the repository, release, compatible asset, and checksum source
    /// without downloading the asset itself.
    pub fn plan(&self, request: InstallRequest) -> Result<InstallPlan> {
        let release = if let Some(tag) = &request.tag {
            self.client.release_by_tag(&request.repo, tag)?
        } else {
            self.client
                .latest_release(&request.repo, self.config.include_prereleases)?
        };
        let choices =
            release::select_assets(self.platform, &release, request.asset_glob.as_deref())?;
        let choice = if let Some(name) = &request.asset_name {
            release::asset_by_name(&choices, name).ok_or_else(|| Error::NoAsset {
                platform: format!("selected asset {name} is not compatible"),
            })?
        } else {
            release::best_asset(&choices)?
        }
        .clone();
        build_plan(
            self.config,
            self.platform,
            &self.bin_dir,
            request,
            release,
            choice,
        )
    }

    /// Download, verify, extract, and atomically install the planned release.
    pub fn install(
        &self,
        plan: &InstallPlan,
        mut progress: impl FnMut(ProgressEvent),
    ) -> Result<InstallOutcome> {
        let temp = tempfile::Builder::new()
            .prefix("harbor-")
            .tempdir()
            .map_err(|error| Error::msg(format!("cannot create temporary directory: {error}")))?;
        let allow_unverified = plan.request.allow_unverified || self.config.allow_unverified;
        let max_bytes = self
            .config
            .max_download_mb
            .saturating_mul(1024 * 1024)
            .max(1);

        let mut expected_digest = None;
        if !plan.checksum_assets.is_empty() {
            for name in &plan.checksum_assets {
                let Some(asset) = plan.release.assets.iter().find(|asset| asset.name == *name)
                else {
                    continue;
                };
                let checksum_download = download::download(
                    self.client.download_http(),
                    self.client.token(),
                    &asset.browser_download_url,
                    temp.path(),
                    1024 * 1024,
                    |current, total| {
                        progress(ProgressEvent {
                            stage: InstallStage::Checksum,
                            current,
                            total,
                        });
                    },
                )?;
                let text = read_text_file(&checksum_download.path, 1024 * 1024)?;
                let entries = checksum::parse_checksums(&text);
                let digest = checksum::expected_digest(&entries, &plan.choice.asset.name)?;
                let _ = fs::remove_file(&checksum_download.path);
                if let Some(digest) = digest {
                    expected_digest = Some(digest);
                    break;
                }
            }
            if expected_digest.is_none() && !allow_unverified {
                return Err(Error::ChecksumInvalid(format!(
                    "{} has checksum files but none cover {}",
                    plan.release.tag_name, plan.choice.asset.name
                )));
            }
        } else if !allow_unverified {
            return Err(Error::ChecksumMissing(plan.choice.asset.name.clone()));
        }

        progress(ProgressEvent {
            stage: InstallStage::Downloading,
            current: 0,
            total: Some(plan.choice.asset.size),
        });
        let downloaded = download::download(
            self.client.download_http(),
            self.client.token(),
            &plan.choice.asset.browser_download_url,
            temp.path(),
            max_bytes,
            |current, total| {
                progress(ProgressEvent {
                    stage: InstallStage::Downloading,
                    current,
                    total,
                });
            },
        )?;
        progress(ProgressEvent {
            stage: InstallStage::Verifying,
            current: 0,
            total: None,
        });
        let checksum_verified = if let Some(expected) = &expected_digest {
            if !checksum::verify_digest(&downloaded.sha256, expected) {
                return Err(Error::ChecksumMismatch {
                    asset: plan.choice.asset.name.clone(),
                    expected: expected.clone(),
                    actual: downloaded.sha256.clone(),
                });
            }
            true
        } else {
            false
        };

        archive::verify_kind(&downloaded.path, plan.choice.kind)?;
        progress(ProgressEvent {
            stage: InstallStage::Extracting,
            current: 0,
            total: None,
        });
        let candidate = if plan.choice.kind.is_archive() {
            let extracted = temp.path().join("extracted");
            fs::create_dir(&extracted)?;
            let limits = ExtractLimits {
                max_files: 10_000,
                max_bytes: max_bytes.saturating_mul(4).min(4 * 1024 * 1024 * 1024),
            };
            archive::extract(&downloaded.path, plan.choice.kind, &extracted, limits)?;
            let candidates =
                archive::discover_binaries(&extracted, &plan.request.repo.name, limits)?;
            archive::pick_binary(&candidates, plan.request.binary.as_deref())?
        } else {
            BinaryCandidate {
                path: downloaded.path.clone(),
                relative: PathBuf::from(&plan.request.repo.name),
                name: plan
                    .request
                    .name
                    .clone()
                    .unwrap_or_else(|| platform::executable_name(&plan.request.repo.name)),
                score: 100,
            }
        };

        let target_name = plan
            .request
            .name
            .clone()
            .unwrap_or_else(|| candidate.name.clone());
        let target_name = platform::executable_name(&platform::validate_binary_name(&target_name)?);
        let bin_dir = platform::ensure_bin_dir(&plan.bin_dir)?;
        let in_path = platform::path_contains(&bin_dir);
        let target = bin_dir.join(&target_name);
        if target.parent() != Some(bin_dir.as_path()) {
            return Err(Error::InvalidBinaryName(target_name));
        }

        let binary_sha256 = checksum::sha256_file(&candidate.path)?;
        let _lock = InstallLock::acquire(&bin_dir)?;
        let mut ledger = Ledger::load()?;
        let key = ledger_key(&target);
        let mut already_current = false;
        let mut replace_allowed = plan.request.force;
        if let Ok(metadata) = fs::symlink_metadata(&target) {
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || metadata.len() > 1024 * 1024 * 1024
            {
                return Err(Error::UnsafeTarget(target));
            }
            let current_hash = checksum::sha256_file(&target)?;
            if current_hash == binary_sha256 {
                already_current = true;
            } else if let Some(entry) = ledger.entries.get(&key) {
                if current_hash == entry.binary_sha256 {
                    replace_allowed = true;
                } else if !plan.request.force {
                    return Err(Error::UnmanagedTarget { path: target });
                }
            } else if !plan.request.force {
                return Err(Error::UnmanagedTarget { path: target });
            }
        }

        let mut warning = plan.warning.clone();
        if already_current {
            let outcome_warning = warning.take();
            return Ok(InstallOutcome {
                path: target,
                repo: plan.request.repo.full_name(),
                version: plan.release.tag_name.clone(),
                asset: plan.choice.asset.name.clone(),
                sha256: binary_sha256,
                bytes: 0,
                checksum_verified,
                already_current: true,
                warning: outcome_warning,
            });
        }

        progress(ProgressEvent {
            stage: InstallStage::Installing,
            current: 0,
            total: None,
        });
        install_file(&candidate.path, &target, replace_allowed)?;
        ledger.entries.insert(
            key,
            LedgerEntry {
                repo: plan.request.repo.full_name(),
                version: plan.release.tag_name.clone(),
                asset: plan.choice.asset.name.clone(),
                asset_sha256: downloaded.sha256.clone(),
                binary_sha256: binary_sha256.clone(),
                installed_at: unix_now(),
                bin_dir: bin_dir.display().to_string(),
            },
        );
        ledger.save()?;
        if !in_path {
            warning = Some(format!(
                "{} is not on PATH; add it to your shell PATH",
                platform::compact_path(&bin_dir)
            ));
        }
        if !checksum_verified {
            warning = Some(match warning {
                Some(existing) => {
                    format!("{existing}; installed without a verified SHA-256 checksum")
                }
                None => "installed without a verified SHA-256 checksum".to_owned(),
            });
        }
        progress(ProgressEvent {
            stage: InstallStage::Complete,
            current: downloaded.bytes,
            total: Some(downloaded.bytes),
        });
        Ok(InstallOutcome {
            path: target,
            repo: plan.request.repo.full_name(),
            version: plan.release.tag_name.clone(),
            asset: plan.choice.asset.name.clone(),
            sha256: binary_sha256,
            bytes: downloaded.bytes,
            checksum_verified,
            already_current: false,
            warning,
        })
    }
}

/// Assemble a plan from an already-fetched release and asset choice. The TUI
/// uses this to confirm an install without repeating API requests.
pub fn build_plan(
    _config: &Config,
    _platform: Platform,
    bin_dir: &BinDir,
    request: InstallRequest,
    release: Release,
    choice: AssetChoice,
) -> Result<InstallPlan> {
    let download_url = url::Url::parse(&choice.asset.browser_download_url)?;
    download::validate_download_url(&download_url)?;
    let checksum_names: Vec<String> = checksum::checksum_assets(&release, &choice.asset)
        .iter()
        .map(|asset| asset.name.clone())
        .collect();
    let target_hint = request
        .name
        .clone()
        .unwrap_or_else(|| platform::executable_name(&request.repo.name));
    platform::validate_binary_name(&target_hint)?;
    let mut warning = None;
    if checksum_names.is_empty() {
        warning = Some("no SHA-256 checksum is published for this asset".to_owned());
    }
    if !bin_dir.in_path {
        warning = Some(match warning {
            Some(existing) => format!(
                "{existing}; {} is not on PATH",
                platform::compact_path(&bin_dir.path)
            ),
            None => format!("{} is not on PATH", platform::compact_path(&bin_dir.path)),
        });
    }
    Ok(InstallPlan {
        request,
        release,
        choice,
        checksum_assets: checksum_names,
        bin_dir: bin_dir.path.clone(),
        in_path: bin_dir.in_path,
        target_hint,
        warning,
    })
}

/// Read at most `limit` bytes of text, rejecting invalid UTF-8.
fn read_text_file(path: &Path, limit: u64) -> Result<String> {
    let file = File::open(path)?;
    let mut reader = file.take(limit.saturating_add(1));
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Error::ChecksumInvalid(format!(
            "{} is larger than {} KiB",
            path.display(),
            limit / 1024
        )));
    }
    String::from_utf8(bytes)
        .map_err(|_| Error::ChecksumInvalid(format!("{} is not UTF-8", path.display())))
}

fn install_file(source: &Path, target: &Path, force: bool) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(target)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(Error::UnsafeTarget(target.to_path_buf()));
    }
    let parent = target
        .parent()
        .ok_or_else(|| Error::InvalidBinaryName(target.display().to_string()))?;
    let mut staged = tempfile::Builder::new()
        .prefix(".harbor-install-")
        .tempfile_in(parent)?;
    let mut source_file = File::open(source)?;
    std::io::copy(&mut source_file, &mut staged)?;
    staged.as_file_mut().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        staged
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o755))?;
    }

    let mut backup = None;
    if target.exists() {
        if !force {
            // The caller normally checks this first, but retain the guard for
            // callers that use the lower-level helper.
            return Err(Error::UnmanagedTarget {
                path: target.to_path_buf(),
            });
        }
        let backup_file = tempfile::Builder::new()
            .prefix(".harbor-backup-")
            .tempfile_in(parent)?;
        let path = backup_file.path().to_path_buf();
        backup_file.close().map_err(Error::Io)?;
        fs::rename(target, &path)?;
        backup = Some(path);
    }

    let staged_path = staged.into_temp_path();
    if let Err(error) = staged_path.persist(target) {
        if let Some(backup) = backup {
            let _ = fs::rename(&backup, target);
        }
        return Err(Error::Io(error.error));
    }
    if let Some(backup) = backup {
        let _ = fs::remove_file(backup);
    }
    #[cfg(unix)]
    {
        let _ = File::open(parent).and_then(|directory| directory.sync_all());
    }
    Ok(())
}

/// An install lock keyed by the destination directory. It prevents two harbor
/// processes from racing while choosing and replacing a binary.
struct InstallLock {
    path: PathBuf,
}

impl InstallLock {
    fn acquire(bin_dir: &Path) -> Result<Self> {
        let dir = platform::state_dir().join("locks");
        fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
        }
        let mut hasher = Sha256::new();
        hasher.update(bin_dir.as_os_str().to_string_lossy().as_bytes());
        let key = hex::encode(hasher.finalize());
        let path = dir.join(format!("{}.lock", &key[..16]));
        if let Ok(metadata) = fs::metadata(&path)
            && metadata
                .modified()
                .ok()
                .and_then(|time| time.elapsed().ok())
                .is_some_and(|age| age.as_secs() > 2 * 60 * 60)
        {
            let _ = fs::remove_file(&path);
        }
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let _ = writeln!(file, "pid={} started={}", std::process::id(), unix_now());
                let _ = file.sync_all();
                Ok(Self { path })
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(Error::Locked(path))
            }
            Err(error) => Err(Error::Io(error)),
        }
    }
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Persistent record of binaries installed by harbor.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    pub entries: BTreeMap<String, LedgerEntry>,
}

/// Metadata for one managed binary.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub repo: String,
    pub version: String,
    pub asset: String,
    pub asset_sha256: String,
    pub binary_sha256: String,
    pub installed_at: u64,
    pub bin_dir: String,
}

impl Ledger {
    pub fn load() -> Result<Self> {
        Self::load_from(&Self::default_path())
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(Error::Io(error)),
        };
        if !metadata.is_file() || metadata.len() > 1024 * 1024 {
            return Err(Error::msg(format!(
                "install ledger {} is not a small regular file; move it aside and retry",
                path.display()
            )));
        }
        let bytes = fs::read(path)?;
        let mut ledger: Self = serde_json::from_slice(&bytes).map_err(|error| {
            Error::msg(format!(
                "install ledger {} is invalid ({error}); move it aside and retry",
                path.display()
            ))
        })?;
        if ledger.entries.len() > 10_000 {
            return Err(Error::msg("install ledger is unexpectedly large"));
        }
        ledger.entries.retain(|_, entry| {
            !entry.binary_sha256.is_empty() && !entry.repo.is_empty() && !entry.bin_dir.is_empty()
        });
        Ok(ledger)
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::default_path();
        self.save_to(&path)?;
        Ok(path)
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent_existed = parent.exists();
        fs::create_dir_all(parent)?;
        #[cfg(unix)]
        if !parent_existed {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
        let text = serde_json::to_vec_pretty(self)?;
        let mut temp = tempfile::Builder::new()
            .prefix(".harbor-ledger-")
            .tempfile_in(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temp.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        temp.write_all(&text)?;
        temp.as_file().sync_all()?;
        persist_replace(temp, path).map_err(|error| Error::Io(error.error))?;
        Ok(())
    }

    #[must_use]
    pub fn default_path() -> PathBuf {
        platform::state_dir().join("installs.json")
    }

    #[must_use]
    pub fn get(&self, path: &Path) -> Option<&LedgerEntry> {
        self.entries.get(&ledger_key(path))
    }
}

fn ledger_key(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        text.to_ascii_lowercase()
    } else {
        text.into_owned()
    }
}

fn persist_replace(
    temp: tempfile::NamedTempFile,
    path: &Path,
) -> std::result::Result<(), tempfile::PersistError> {
    #[cfg(windows)]
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    temp.persist(path).map(|_| ())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::Asset;
    use crate::release::AssetKind;

    fn sample_asset(name: &str) -> Asset {
        Asset {
            id: 1,
            name: name.to_owned(),
            size: 1024,
            download_count: None,
            content_type: None,
            browser_download_url: format!(
                "https://github.com/owner/tool/releases/download/v1/{name}"
            ),
        }
    }

    fn sample_release(names: &[&str]) -> Release {
        Release {
            tag_name: "v1.0.0".to_owned(),
            name: None,
            body: None,
            draft: false,
            prerelease: false,
            published_at: None,
            assets: names.iter().map(|name| sample_asset(name)).collect(),
        }
    }

    #[test]
    fn build_plan_detects_checksum_and_warns_about_path() {
        let release = sample_release(&[
            "tool-v1-x86_64-unknown-linux-musl.tar.gz",
            "tool-v1-x86_64-unknown-linux-musl.tar.gz.sha256",
        ]);
        let choice = AssetChoice {
            asset: release.assets[0].clone(),
            kind: AssetKind::TarGz,
            score: 100,
            explicit: false,
        };
        let request = InstallRequest {
            repo: RepoRef {
                owner: "owner".to_owned(),
                name: "tool".to_owned(),
            },
            tag: Some("v1.0.0".to_owned()),
            asset_name: Some(release.assets[0].name.clone()),
            asset_glob: None,
            binary: None,
            name: None,
            force: false,
            allow_unverified: false,
        };
        let bin_dir = BinDir {
            path: PathBuf::from("/tmp/harbor-plan-bin"),
            in_path: false,
        };
        let plan = build_plan(
            &Config::default(),
            Platform::current(),
            &bin_dir,
            request,
            release,
            choice,
        )
        .unwrap();
        assert!(plan.checksum_available());
        assert_eq!(plan.target_hint, platform::executable_name("tool"));
        assert!(plan.warning.is_some());
    }

    #[test]
    fn build_plan_rejects_a_non_github_download_url() {
        let mut release = sample_release(&["tool-v1-x86_64-unknown-linux-musl.tar.gz"]);
        release.assets[0].browser_download_url = "https://evil.example/tool.tar.gz".to_owned();
        let choice = AssetChoice {
            asset: release.assets[0].clone(),
            kind: AssetKind::TarGz,
            score: 100,
            explicit: false,
        };
        let request = InstallRequest {
            repo: RepoRef {
                owner: "owner".to_owned(),
                name: "tool".to_owned(),
            },
            tag: None,
            asset_name: None,
            asset_glob: None,
            binary: None,
            name: None,
            force: false,
            allow_unverified: false,
        };
        let bin_dir = BinDir {
            path: PathBuf::from("/tmp/harbor-plan-bin"),
            in_path: true,
        };
        let error = build_plan(
            &Config::default(),
            Platform::current(),
            &bin_dir,
            request,
            release,
            choice,
        )
        .unwrap_err();
        assert!(matches!(error, Error::UntrustedHost(_)));
    }

    #[test]
    fn install_file_rejects_a_symlink_target() {
        #[cfg(unix)]
        {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source");
            let real = temp.path().join("real");
            let link = temp.path().join("link");
            fs::write(&source, "new").unwrap();
            fs::write(&real, "old").unwrap();
            std::os::unix::fs::symlink(&real, &link).unwrap();
            assert!(matches!(
                install_file(&source, &link, true),
                Err(Error::UnsafeTarget(_))
            ));
            assert_eq!(fs::read_to_string(&real).unwrap(), "old");
        }
    }

    #[test]
    fn ledger_round_trips() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ledger.json");
        let mut ledger = Ledger::default();
        ledger.entries.insert(
            "/tmp/bin/tool".to_owned(),
            LedgerEntry {
                repo: "owner/tool".to_owned(),
                version: "v1".to_owned(),
                asset: "tool.tar.gz".to_owned(),
                asset_sha256: "a".repeat(64),
                binary_sha256: "b".repeat(64),
                installed_at: 1,
                bin_dir: "/tmp/bin".to_owned(),
            },
        );
        ledger.save_to(&path).unwrap();
        let loaded = Ledger::load_from(&path).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(
            loaded.get(Path::new("/tmp/bin/tool")).unwrap().version,
            "v1"
        );
    }

    #[test]
    fn invalid_ledger_is_not_silently_accepted() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("ledger.json");
        fs::write(&path, "not json").unwrap();
        assert!(Ledger::load_from(&path).is_err());
    }

    #[test]
    fn install_file_requires_force_for_unmanaged_files() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::write(&source, "new").unwrap();
        fs::write(&target, "old").unwrap();
        assert!(matches!(
            install_file(&source, &target, false),
            Err(Error::UnmanagedTarget { .. })
        ));
        assert_eq!(fs::read_to_string(&target).unwrap(), "old");
        install_file(&source, &target, true).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
    }
}
