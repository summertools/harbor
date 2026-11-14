//! Persistent configuration and source definitions.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::platform;

const MAX_CONFIG_BYTES: u64 = 128 * 1024;
const MAX_SOURCES: usize = 64;

/// Where repositories are discovered from.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    /// Every public repository under a GitHub organisation (or user).
    Org,
    /// A single `owner/repo` GitHub repository.
    Repo,
}

impl SourceKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Org => "org",
            Self::Repo => "repo",
        }
    }
}

/// One configured discovery source.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Source {
    #[serde(rename = "type")]
    pub kind: SourceKind,
    pub name: String,
}

impl Source {
    /// Create and validate an organisation source.
    pub fn org(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        validate_owner(&name)?;
        Ok(Self {
            kind: SourceKind::Org,
            name,
        })
    }

    /// Create and validate a repository source.
    pub fn repo(full_name: impl Into<String>) -> Result<Self> {
        let full_name = full_name.into();
        validate_repo_full_name(&full_name)?;
        Ok(Self {
            kind: SourceKind::Repo,
            name: full_name,
        })
    }

    /// Parse `org:name`, `repo:owner/repo`, or a bare source. A bare value
    /// containing `/` is treated as a repository; otherwise it is an org.
    pub fn parse_spec(spec: &str) -> Result<Self> {
        let spec = spec.trim();
        if spec.is_empty() {
            return Err(Error::Source("source cannot be empty".to_owned()));
        }
        if spec.eq_ignore_ascii_case("org") || spec.eq_ignore_ascii_case("repo") {
            return Err(Error::Source(format!("{spec} source is missing a name")));
        }
        if let Some(rest) = spec.strip_prefix("org:") {
            return Self::org(rest.trim());
        }
        if let Some(rest) = spec.strip_prefix("repo:") {
            return Self::repo(rest.trim());
        }
        if let Some(rest) = spec.strip_prefix("org ") {
            return Self::org(rest.trim());
        }
        if let Some(rest) = spec.strip_prefix("repo ") {
            return Self::repo(rest.trim());
        }
        if spec.contains('/') {
            Self::repo(spec)
        } else {
            Self::org(spec)
        }
    }

    #[must_use]
    pub fn label(&self) -> String {
        format!("{}:{}", self.kind.label(), self.name)
    }

    #[must_use]
    pub fn matches(&self, value: &str) -> bool {
        self.label().eq_ignore_ascii_case(value) || self.name.eq_ignore_ascii_case(value)
    }
}

/// User configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Optional install directory. CLI/environment override this.
    pub bin_dir: Option<PathBuf>,
    /// Permit assets without a published SHA-256 digest. Off by default.
    pub allow_unverified: bool,
    /// Consider prerelease tags when a repository has no stable release.
    pub include_prereleases: bool,
    /// Automatically update managed installs when a newer, compatible,
    /// checksum-verified release is available. Off by default.
    pub auto_update: bool,
    /// Safety cap for one downloaded asset.
    #[serde(default = "default_max_download_mb")]
    pub max_download_mb: u64,
    /// Discovery sources. The default is `org:summertools`.
    #[serde(default = "default_sources")]
    pub sources: Vec<Source>,
}

fn default_sources() -> Vec<Source> {
    vec![Source::org("summertools").expect("built-in source is valid")]
}

const fn default_max_download_mb() -> u64 {
    512
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bin_dir: None,
            allow_unverified: false,
            include_prereleases: false,
            auto_update: false,
            max_download_mb: default_max_download_mb(),
            sources: default_sources(),
        }
    }
}

impl Config {
    /// Load the config. A broken implicitly-discovered config falls back to
    /// defaults with a warning; an explicit `--config`/`HARBOR_CONFIG` error is
    /// reported so scripts never silently use the wrong sources.
    pub fn load(explicit: Option<&Path>) -> Result<(Self, Vec<String>)> {
        let mut warnings = Vec::new();
        let env_path = env::var_os("HARBOR_CONFIG")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let (path, strict) = if let Some(path) = explicit {
            (Some(path.to_path_buf()), true)
        } else if let Some(path) = env_path {
            (Some(path), true)
        } else {
            (Some(default_config_path()), false)
        };
        let Some(path) = path else {
            return Ok((Self::default(), warnings));
        };

        let bytes = match fs::metadata(&path) {
            Ok(metadata) if !metadata.is_file() => {
                let message = format!("{} is not a regular file", path.display());
                if strict {
                    return Err(Error::Config(message));
                }
                warnings.push(message);
                return Ok((Self::default(), warnings));
            }
            Ok(metadata) if metadata.len() > MAX_CONFIG_BYTES => {
                let message = format!(
                    "{} is larger than {} KiB",
                    path.display(),
                    MAX_CONFIG_BYTES / 1024
                );
                if strict {
                    return Err(Error::Config(message));
                }
                warnings.push(message);
                return Ok((Self::default(), warnings));
            }
            Ok(_) => match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    let message = format!("cannot read {}: {error}", path.display());
                    if strict {
                        return Err(Error::Config(message));
                    }
                    warnings.push(message);
                    return Ok((Self::default(), warnings));
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let message = format!(
                    "{} does not exist yet; using defaults (it will be created when saved)",
                    path.display()
                );
                if strict {
                    warnings.push(message);
                }
                return Ok((Self::default(), warnings));
            }
            Err(error) => {
                let message = format!("cannot read {}: {error}", path.display());
                if strict {
                    return Err(Error::Config(message));
                }
                warnings.push(message);
                return Ok((Self::default(), warnings));
            }
        };

        let text = std::str::from_utf8(&bytes)
            .map_err(|_| Error::Config(format!("{} is not UTF-8", path.display())))?;
        let mut config: Self = match toml::from_str(text) {
            Ok(config) => config,
            Err(error) => {
                let message = format!("invalid config {}: {error}", path.display());
                if strict {
                    return Err(Error::Config(message));
                }
                warnings.push(message);
                return Ok((Self::default(), warnings));
            }
        };
        config.normalise(&mut warnings);
        Ok((config, warnings))
    }

    fn normalise(&mut self, warnings: &mut Vec<String>) {
        if self.max_download_mb == 0 {
            warnings.push("max_download_mb must be at least 1; using 512".to_owned());
            self.max_download_mb = 512;
        } else if self.max_download_mb > 4096 {
            warnings.push("max_download_mb capped at 4096 MiB".to_owned());
            self.max_download_mb = 4096;
        }

        let mut seen = HashSet::new();
        self.sources.retain(|source| {
            if let Err(error) = validate_source(source) {
                warnings.push(format!("ignored invalid source: {error}"));
                return false;
            }
            let key = source.label().to_ascii_lowercase();
            if !seen.insert(key) {
                warnings.push(format!("ignored duplicate source {}", source.label()));
                return false;
            }
            true
        });
        if self.sources.len() > MAX_SOURCES {
            self.sources.truncate(MAX_SOURCES);
            warnings.push(format!("source list capped at {MAX_SOURCES} entries"));
        }
        if self.sources.is_empty() {
            warnings.push(
                "no sources configured; add one with `harbor source add org <name>`".to_owned(),
            );
        }
    }

    /// Persist to the default config path.
    pub fn save(&self) -> Result<PathBuf> {
        let path = default_config_path();
        self.save_to(&path)?;
        Ok(path)
    }

    /// Persist to an explicit path using a same-directory atomic replace.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        let mut copy = self.clone();
        let mut warnings = Vec::new();
        copy.normalise(&mut warnings);
        let text = toml::to_string_pretty(&copy)
            .map_err(|error| Error::Config(format!("cannot serialise config: {error}")))?;
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
        let mut temp = tempfile::Builder::new()
            .prefix(".harbor-config-")
            .tempfile_in(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temp.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        temp.write_all(text.as_bytes())?;
        temp.as_file().sync_all()?;
        persist_replace(temp, path).map_err(|error| Error::Io(error.error))?;
        Ok(())
    }

    /// Add a source if it is not already present. Returns the canonical label.
    pub fn add_source(&mut self, source: Source) -> Result<String> {
        validate_source(&source)?;
        if self
            .sources
            .iter()
            .any(|existing| existing.label().eq_ignore_ascii_case(&source.label()))
        {
            return Err(Error::Source(format!(
                "source {} is already configured",
                source.label()
            )));
        }
        if self.sources.len() >= MAX_SOURCES {
            return Err(Error::Source(format!(
                "at most {MAX_SOURCES} sources can be configured"
            )));
        }
        let label = source.label();
        self.sources.push(source);
        Ok(label)
    }

    /// Remove a source by label or name. Returns true when something was removed.
    pub fn remove_source(&mut self, value: &str) -> bool {
        let value = value.trim();
        let before = self.sources.len();
        self.sources.retain(|source| !source.matches(value));
        before != self.sources.len()
    }

    /// Find a source by its label or bare name.
    #[must_use]
    pub fn find_source(&self, value: &str) -> Option<&Source> {
        self.sources.iter().find(|source| source.matches(value))
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

fn validate_source(source: &Source) -> Result<()> {
    match source.kind {
        SourceKind::Org => validate_owner(&source.name),
        SourceKind::Repo => validate_repo_full_name(&source.name),
    }
}

fn validate_owner(owner: &str) -> Result<()> {
    let owner = owner.trim();
    if owner.is_empty() || owner.len() > 39 {
        return Err(Error::Source(
            "organization name must be 1-39 characters".to_owned(),
        ));
    }
    if owner.starts_with('-') || owner.ends_with('-') {
        return Err(Error::Source(
            "organization name cannot start or end with '-'".to_owned(),
        ));
    }
    if !owner
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(Error::Source(format!(
            "invalid organization name {owner:?}"
        )));
    }
    Ok(())
}

fn validate_repo_full_name(full_name: &str) -> Result<()> {
    let full_name = full_name.trim();
    let Some((owner, repo)) = full_name.split_once('/') else {
        return Err(Error::Source(
            "repository source must be owner/repo".to_owned(),
        ));
    };
    if repo.contains('/') {
        return Err(Error::Source(
            "repository source must contain exactly one '/'".to_owned(),
        ));
    }
    validate_owner(owner)?;
    if repo.is_empty() || repo.len() > 100 {
        return Err(Error::Source(
            "repository name must be 1-100 characters".to_owned(),
        ));
    }
    if repo == "." || repo == ".." || repo.ends_with(".git") {
        return Err(Error::Source(format!("invalid repository name {repo:?}")));
    }
    if !repo
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(Error::Source(format!("invalid repository name {repo:?}")));
    }
    Ok(())
}

/// Verify an external repository reference before using it in an API path.
pub fn parse_repo_ref(full_name: &str) -> Result<(String, String)> {
    let full_name = full_name.trim();
    let Some((owner, repo)) = full_name.split_once('/') else {
        return Err(Error::Source(
            "repository must be owner/repo (or a unique name from a configured org)".to_owned(),
        ));
    };
    if repo.contains('/') {
        return Err(Error::Source(
            "repository must contain exactly one '/'".to_owned(),
        ));
    }
    validate_owner(owner)?;
    validate_repo_full_name(full_name)?;
    Ok((owner.to_owned(), repo.to_owned()))
}

#[must_use]
pub fn default_config_path() -> PathBuf {
    platform::config_dir().join("config.toml")
}

#[must_use]
pub fn display_config_path(path: &Path) -> String {
    platform::compact_path(path)
}

/// Convenience for tests and callers that need a config path under a custom
/// XDG directory.
#[must_use]
pub fn config_path_in(dir: &Path) -> PathBuf {
    dir.join("harbor").join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_specs_and_validation() {
        assert_eq!(
            Source::parse_spec("summertools").unwrap(),
            Source::org("summertools").unwrap()
        );
        assert_eq!(
            Source::parse_spec("repo:BurntSushi/ripgrep").unwrap(),
            Source::repo("BurntSushi/ripgrep").unwrap()
        );
        assert_eq!(
            Source::parse_spec("repo BurntSushi/ripgrep").unwrap(),
            Source::repo("BurntSushi/ripgrep").unwrap()
        );
        assert_eq!(
            Source::parse_spec("org summertools").unwrap(),
            Source::org("summertools").unwrap()
        );
        assert!(Source::parse_spec("owner/").is_err());
        assert!(Source::parse_spec("owner/repo/extra").is_err());
        assert!(Source::parse_spec("bad name").is_err());
        assert!(Source::parse_spec("owner/repo;rm").is_err());
    }

    #[test]
    fn default_config_targets_summertools_and_disables_auto_update() {
        let config = Config::default();
        assert_eq!(config.sources, vec![Source::org("summertools").unwrap()]);
        assert!(!config.auto_update);
    }

    #[test]
    fn add_and_remove_sources_is_case_insensitive() {
        let mut config = Config::default();
        config
            .add_source(Source::repo("Owner/Repo").unwrap())
            .unwrap();
        assert!(
            config
                .add_source(Source::repo("owner/repo").unwrap())
                .is_err()
        );
        assert!(config.remove_source("OWNER/REPO"));
        assert!(!config.remove_source("OWNER/REPO"));
    }

    #[test]
    fn malformed_explicit_config_is_an_error_but_missing_config_uses_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let malformed = temp.path().join("bad.toml");
        fs::write(&malformed, "not = [toml").unwrap();
        assert!(Config::load(Some(&malformed)).is_err());

        let missing = temp.path().join("missing.toml");
        let (config, warnings) = Config::load(Some(&missing)).unwrap();
        assert_eq!(config.sources, Config::default().sources);
        assert!(!warnings.is_empty());
    }

    #[test]
    fn missing_fields_keep_safe_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("partial.toml");
        fs::write(&path, "bin_dir = \"/tmp/bin\"\n").unwrap();
        let (config, warnings) = Config::load(Some(&path)).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(config.sources, Config::default().sources);
        assert_eq!(config.max_download_mb, 512);
        assert!(!config.allow_unverified);
    }

    #[test]
    fn config_round_trips_through_toml() {
        let mut config = Config::default();
        config
            .add_source(Source::repo("BurntSushi/ripgrep").unwrap())
            .unwrap();
        let text = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.sources.len(), 2);
        assert_eq!(parsed.max_download_mb, 512);
    }
}
