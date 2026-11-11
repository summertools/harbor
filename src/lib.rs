//! Harbor: a safe, interactive installer for GitHub release binaries.
//!
//! The library layers are intentionally independent:
//!
//! * [`config`] persists discovery sources and safety settings.
//! * [`github`] talks to the GitHub REST API with bounded responses.
//! * [`release`] and [`checksum`] choose a compatible asset and digest.
//! * [`archive`] extracts archives without path traversal or symlinks.
//! * [`install`] verifies, stages, and atomically replaces binaries.
//!
//! The TUI is a thin consumer of the same APIs, so every install path goes
//! through the same verification and overwrite protection.

pub mod archive;
pub mod checksum;
pub mod cli;
pub mod config;
pub mod download;
pub mod error;
pub mod format;
pub mod github;
pub mod install;
pub mod output;
pub mod platform;
pub mod release;
pub mod ui;

pub use config::{Config, Source, SourceKind};
pub use error::{Error, Result};
pub use github::{Asset, GitHubClient, Release, RepoRef, RepoSummary};
pub use install::{
    InstallOutcome, InstallPlan, InstallRequest, InstallStage, Installer, Ledger, LedgerEntry,
    ProgressEvent,
};
pub use platform::{BinDir, Platform};

use std::collections::HashSet;
use std::path::Path;

/// A repository entry discovered from one configured source.
#[derive(Clone, Debug)]
pub struct RepoItem {
    pub source: Source,
    pub full_name: String,
    pub name: String,
    pub description: Option<String>,
    pub archived: bool,
    pub fork: bool,
    pub html_url: Option<String>,
}

impl RepoItem {
    #[must_use]
    pub fn repo_ref(&self) -> RepoRef {
        RepoRef {
            owner: self
                .full_name
                .split_once('/')
                .map_or("", |(owner, _)| owner)
                .to_owned(),
            name: self.name.clone(),
        }
    }
}

/// Result of discovering all configured sources.
#[derive(Clone, Debug, Default)]
pub struct Discovery {
    pub repos: Vec<RepoItem>,
    pub warnings: Vec<String>,
}

/// High-level facade used by the CLI and TUI.
pub struct Harbor {
    pub config: Config,
    pub platform: Platform,
    pub bin_dir: BinDir,
    pub client: GitHubClient,
}

impl Harbor {
    /// Build a facade with a resolved executable directory.
    pub fn new(config: Config, bin_dir_override: Option<&Path>) -> Result<Self> {
        let bin_dir = platform::resolve_bin_dir(bin_dir_override, config.bin_dir.as_deref())?;
        let client = GitHubClient::new()?;
        Ok(Self {
            config,
            platform: Platform::current(),
            bin_dir,
            client,
        })
    }

    /// Build a facade without creating the install directory. Used by dry-run
    /// commands so inspecting a plan has no filesystem side effects.
    pub fn planned(config: Config, bin_dir_override: Option<&Path>) -> Result<Self> {
        let bin_dir = platform::planned_bin_dir(bin_dir_override, config.bin_dir.as_deref())?;
        let client = GitHubClient::new()?;
        Ok(Self {
            config,
            platform: Platform::current(),
            bin_dir,
            client,
        })
    }

    /// Build with an explicit client, useful for tests and embedders.
    pub fn with_client(
        config: Config,
        bin_dir_override: Option<&Path>,
        client: GitHubClient,
    ) -> Result<Self> {
        let bin_dir = platform::resolve_bin_dir(bin_dir_override, config.bin_dir.as_deref())?;
        Ok(Self {
            config,
            platform: Platform::current(),
            bin_dir,
            client,
        })
    }

    /// Discover repositories from every configured source.
    pub fn discover(&self) -> Discovery {
        discover(&self.config, &self.client)
    }

    /// Resolve `owner/repo`, or a unique bare repository name from a configured
    /// org source. Ambiguous short names return a useful error listing matches.
    pub fn resolve_repo(&self, value: &str) -> Result<RepoRef> {
        resolve_repo(&self.config, &self.client, value)
    }

    /// Fetch the latest release for a repository.
    pub fn latest_release(&self, repo: &RepoRef) -> Result<Release> {
        self.client
            .latest_release(repo, self.config.include_prereleases)
    }

    /// Build an install plan without downloading the asset.
    pub fn plan(&self, request: InstallRequest) -> Result<InstallPlan> {
        Installer::new(&self.client, &self.config, self.bin_dir.clone()).plan(request)
    }

    /// Plan and install in one call.
    pub fn install(
        &self,
        request: InstallRequest,
        progress: impl FnMut(ProgressEvent),
    ) -> Result<InstallOutcome> {
        let plan = self.plan(request)?;
        Installer::new(&self.client, &self.config, self.bin_dir.clone()).install(&plan, progress)
    }

    #[must_use]
    pub fn installer(&self) -> Installer<'_> {
        Installer::new(&self.client, &self.config, self.bin_dir.clone())
    }
}

/// Discover repositories from every configured source without resolving an
/// executable directory. Useful for read-only commands such as `list`.
pub fn discover(config: &Config, client: &GitHubClient) -> Discovery {
    let mut discovery = Discovery::default();
    let mut seen = HashSet::new();
    for source in &config.sources {
        match source.kind {
            SourceKind::Org => match client.list_org_repos(&source.name, 200) {
                Ok(repos) => {
                    for repo in repos {
                        if !seen.insert(repo.full_name.to_ascii_lowercase()) {
                            continue;
                        }
                        discovery.repos.push(RepoItem {
                            source: source.clone(),
                            full_name: repo.full_name,
                            name: repo.name,
                            description: repo.description,
                            archived: repo.archived,
                            fork: repo.fork,
                            html_url: repo.html_url,
                        });
                    }
                }
                Err(error) => discovery
                    .warnings
                    .push(format!("source {}: {error}", source.label())),
            },
            SourceKind::Repo => {
                let Ok(repo) = RepoRef::parse(&source.name) else {
                    discovery.warnings.push(format!(
                        "source {} has an invalid repository name",
                        source.label()
                    ));
                    continue;
                };
                if !seen.insert(repo.full_name().to_ascii_lowercase()) {
                    continue;
                }
                let (description, archived, fork, html_url) = match client.repo(&repo) {
                    Ok(info) => (info.description, info.archived, info.fork, info.html_url),
                    Err(error) => {
                        discovery
                            .warnings
                            .push(format!("source {}: {error}", source.label()));
                        (None, false, false, None)
                    }
                };
                discovery.repos.push(RepoItem {
                    source: source.clone(),
                    full_name: repo.full_name(),
                    name: repo.name,
                    description,
                    archived,
                    fork,
                    html_url,
                });
            }
        }
    }
    discovery.repos.sort_by(|left, right| {
        left.archived.cmp(&right.archived).then_with(|| {
            left.full_name
                .to_ascii_lowercase()
                .cmp(&right.full_name.to_ascii_lowercase())
        })
    });
    discovery
}

/// Resolve a repository using the configured sources.
pub fn resolve_repo(config: &Config, client: &GitHubClient, value: &str) -> Result<RepoRef> {
    if value.contains('/') {
        return RepoRef::parse(value);
    }
    let wanted = value.trim().to_ascii_lowercase();
    if wanted.is_empty() {
        return Err(Error::Source("repository name is empty".to_owned()));
    }
    let discovery = discover(config, client);
    let matches: Vec<&RepoItem> = discovery
        .repos
        .iter()
        .filter(|repo| repo.name.eq_ignore_ascii_case(&wanted))
        .collect();
    match matches.as_slice() {
        [] => Err(Error::NotFound(format!(
            "no configured repository named {value:?}{}",
            short_warning(&discovery.warnings)
        ))),
        [repo] => Ok(repo.repo_ref()),
        many => {
            let names = many
                .iter()
                .map(|repo| repo.full_name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Err(Error::Source(format!(
                "repository name {value:?} is ambiguous; use one of: {names}"
            )))
        }
    }
}

fn short_warning(warnings: &[String]) -> String {
    warnings
        .first()
        .map(|warning| format!(" ({warning})"))
        .unwrap_or_default()
}
