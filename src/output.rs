//! Human-readable non-interactive output.

use std::collections::HashMap;
use std::env;
use std::io::{self, Write};

use serde::Serialize;

use crate::Discovery;
use crate::format;
use crate::github::{Release, RepoRef};
use crate::install::{InstallOutcome, InstallPlan, InstalledPackage};
use crate::platform::Platform;
use crate::release;

/// Small ANSI painter with an explicit on/off switch.
#[derive(Clone, Copy, Debug)]
pub struct Painter {
    pub enabled: bool,
}

impl Painter {
    #[must_use]
    pub const fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    fn paint(self, code: &str, text: &str) -> String {
        if !self.enabled || text.is_empty() {
            text.to_owned()
        } else {
            format!("\x1b[{code}m{text}\x1b[0m")
        }
    }

    #[must_use]
    pub fn title(self, text: &str) -> String {
        self.paint("1;36", text)
    }

    #[must_use]
    pub fn accent(self, text: &str) -> String {
        self.paint("36", text)
    }

    #[must_use]
    pub fn good(self, text: &str) -> String {
        self.paint("1;32", text)
    }

    #[must_use]
    pub fn warning(self, text: &str) -> String {
        self.paint("1;33", text)
    }

    #[must_use]
    pub fn error(self, text: &str) -> String {
        self.paint("1;31", text)
    }

    #[must_use]
    pub fn dim(self, text: &str) -> String {
        self.paint("2", text)
    }

    #[must_use]
    pub fn bold(self, text: &str) -> String {
        self.paint("1", text)
    }
}

#[must_use]
pub fn terminal_width() -> usize {
    env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|width| *width >= 40)
        .unwrap_or(100)
        .min(240)
}

pub fn print_warnings(warnings: &[String], painter: Painter) {
    for warning in warnings {
        eprintln!(
            "{} {}",
            painter.warning("warning:"),
            format::sanitize(warning)
        );
    }
}

#[derive(Serialize)]
struct RepoJson<'a> {
    source: String,
    full_name: &'a str,
    description: Option<&'a str>,
    archived: bool,
    fork: bool,
    html_url: Option<&'a str>,
    installed: Option<InstalledJson<'a>>,
}

#[derive(Serialize)]
struct InstalledJson<'a> {
    version: &'a str,
    path: String,
    installed_at: u64,
    binaries: usize,
    present: bool,
}

pub fn print_repos(
    discovery: &Discovery,
    show_archived: bool,
    json: bool,
    painter: Painter,
    installed: &[InstalledPackage],
) {
    let installed: HashMap<String, &InstalledPackage> = installed
        .iter()
        .map(|package| (package.repo.to_ascii_lowercase(), package))
        .collect();
    if json {
        let rows: Vec<RepoJson<'_>> = discovery
            .repos
            .iter()
            .filter(|repo| show_archived || !repo.archived)
            .map(|repo| RepoJson {
                source: repo.display_source(),
                full_name: &repo.full_name,
                description: repo.description.as_deref(),
                archived: repo.archived,
                fork: repo.fork,
                html_url: repo.html_url.as_deref(),
                installed: installed
                    .get(repo.full_name.as_str())
                    .map(|package| InstalledJson {
                        version: &package.version,
                        path: package.display_path(),
                        installed_at: package.installed_at,
                        binaries: package.binaries,
                        present: package.present,
                    }),
            })
            .collect();
        print_json(&rows);
        return;
    }
    let width = terminal_width();
    let mut shown = 0usize;
    for repo in discovery
        .repos
        .iter()
        .filter(|repo| show_archived || !repo.archived)
    {
        shown += 1;
        let badge = painter.dim(&format!("[{}]", repo.display_source()));
        let raw_name = format::sanitize(&repo.full_name);
        let name_column = 30.min(width / 3).max(14);
        let name = format::pad_right(&format::truncate(&raw_name, name_column), name_column);
        let package = installed.get(&repo.full_name.to_ascii_lowercase()).copied();
        let status_column = 20.min(width / 4).max(10);
        let status_rendered = match package {
            Some(package) if !package.present => {
                let suffix = format!("missing {}", package.version);
                painter.warning(&format::pad_right(
                    &format::truncate(&suffix, status_column),
                    status_column,
                ))
            }
            Some(package) => {
                let suffix = format!("installed {}", package.version);
                painter.good(&format::pad_right(
                    &format::truncate(&suffix, status_column),
                    status_column,
                ))
            }
            None => painter.dim(&format::pad_right(
                &format::truncate("not installed", status_column),
                status_column,
            )),
        };
        let mut tags = String::new();
        let mut tags_width = 0usize;
        if repo.archived {
            tags.push_str(&painter.warning(" archived"));
            tags_width += " archived".len();
        }
        if repo.fork {
            tags.push_str(&painter.dim(" fork"));
            tags_width += " fork".len();
        }
        let used =
            name_column + format::width(&repo.display_source()) + tags_width + status_column + 10;
        let description_width = width.saturating_sub(used);
        let description = repo
            .description
            .as_deref()
            .map(format::sanitize)
            .unwrap_or_default();
        if description_width >= 8 {
            let description = format::truncate(&description, description_width);
            println!(
                "{} {}  {}  {}{}",
                badge,
                painter.bold(&name),
                status_rendered,
                painter.dim(&description),
                tags
            );
        } else {
            println!(
                "{} {}  {}{}",
                badge,
                painter.bold(&name),
                status_rendered,
                tags
            );
        }
    }
    if shown == 0 {
        println!("{}", painter.dim("no repositories found"));
    } else {
        println!(
            "\n{}",
            painter.dim(&format!(
                "{shown} repositor{}",
                if shown == 1 { "y" } else { "ies" }
            ))
        );
    }
}

#[derive(Serialize)]
struct ReleaseJson<'a> {
    tag_name: &'a str,
    name: &'a str,
    published_at: Option<&'a str>,
    prerelease: bool,
    assets: &'a [crate::github::Asset],
}

pub fn print_releases(
    repo: &RepoRef,
    releases: &[Release],
    platform: Platform,
    json: bool,
    painter: Painter,
) {
    if json {
        let rows: Vec<ReleaseJson<'_>> = releases
            .iter()
            .map(|release| ReleaseJson {
                tag_name: &release.tag_name,
                name: release.display_name(),
                published_at: release.published_at.as_deref(),
                prerelease: release.prerelease,
                assets: &release.assets,
            })
            .collect();
        print_json(&rows);
        return;
    }
    println!(
        "{}  {}",
        painter.title(&format::sanitize(&repo.full_name())),
        painter.dim(&format!("({})", platform.label()))
    );
    let name_width = terminal_width().saturating_sub(22).clamp(18, 62);
    for release in releases {
        let tag = if release.prerelease {
            painter.warning(&format::sanitize(&release.tag_name))
        } else {
            painter.accent(&format::sanitize(&release.tag_name))
        };
        println!(
            "\n{} {}  {}",
            tag,
            painter.bold(&format::sanitize(release.display_name())),
            painter.dim(release.date())
        );
        match release::select_assets(platform, release, None) {
            Ok(choices) => {
                for choice in choices.iter().take(12) {
                    let marker = if choice.explicit {
                        painter.good("*")
                    } else {
                        painter.good("›")
                    };
                    let asset_name = format::sanitize(&choice.asset.name);
                    let name = format::truncate(&asset_name, name_width);
                    println!(
                        "  {} {:<name_width$} {:>10}  {}",
                        marker,
                        name,
                        format::bytes(choice.asset.size),
                        painter.dim(choice.kind.label())
                    );
                }
                if choices.len() > 12 {
                    println!(
                        "  {}",
                        painter.dim(&format!("… and {} more", choices.len() - 12))
                    );
                }
            }
            Err(error) => {
                println!(
                    "  {}",
                    painter.warning(&format!("no compatible asset: {error}"))
                );
            }
        }
    }
}

pub fn print_plan(plan: &InstallPlan, painter: Painter) {
    println!("{}", painter.title("Install plan"));
    println!(
        "  {:<12} {}",
        painter.dim("repository"),
        painter.bold(&plan.request.repo.full_name())
    );
    println!(
        "  {:<12} {} {}",
        painter.dim("release"),
        painter.accent(&format::sanitize(&plan.release.tag_name)),
        painter.dim(&format!("({})", plan.release.date()))
    );
    println!(
        "  {:<12} {}  {}",
        painter.dim("asset"),
        format::sanitize(&plan.choice.asset.name),
        painter.dim(&format::bytes(plan.choice.asset.size))
    );
    println!(
        "  {:<12} {}",
        painter.dim("checksum"),
        if plan.checksum_available() {
            painter.good("SHA-256 asset published")
        } else {
            painter.warning("not published")
        }
    );
    println!(
        "  {:<12} {}",
        painter.dim("install dir"),
        format::sanitize(&plan.bin_dir.display().to_string())
    );
    if let Some(warning) = &plan.warning {
        println!(
            "  {} {}",
            painter.warning("warning:"),
            format::sanitize(warning)
        );
    }
}

pub fn print_outcome(outcome: &InstallOutcome, painter: Painter) {
    if outcome.already_current {
        println!(
            "{} {} is already up to date at {}",
            painter.good("✓"),
            painter.bold(&outcome.repo),
            format::sanitize(&outcome.path.display().to_string())
        );
    } else {
        println!(
            "{} installed {} {} → {}",
            painter.good("✓"),
            painter.bold(&outcome.repo),
            painter.accent(&format::sanitize(&outcome.version)),
            format::sanitize(&outcome.path.display().to_string())
        );
        println!(
            "  {}  {}",
            painter.dim(&format::bytes(outcome.bytes)),
            if outcome.checksum_verified {
                painter.good("SHA-256 verified")
            } else {
                painter.warning("SHA-256 not verified")
            }
        );
    }
    if let Some(warning) = &outcome.warning {
        eprintln!(
            "{} {}",
            painter.warning("warning:"),
            format::sanitize(warning)
        );
    }
}

/// One row in the installed-status table.
#[derive(Clone, Debug, Serialize)]
pub struct StatusEntry {
    pub repo: String,
    pub installed: String,
    pub latest: Option<String>,
    pub state: StatusState,
    pub path: String,
    pub binaries: usize,
    pub installed_at: u64,
    pub present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StatusState {
    UpToDate,
    UpdateAvailable,
    Installed,
    Missing,
    LatestUnavailable,
    NoCompatibleAsset,
}

impl StatusState {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::UpToDate => "up to date",
            Self::UpdateAvailable => "update available",
            Self::Installed => "installed",
            Self::Missing => "missing",
            Self::LatestUnavailable => "latest unavailable",
            Self::NoCompatibleAsset => "no compatible asset",
        }
    }
}

impl StatusEntry {
    #[must_use]
    pub fn from_package(
        package: &InstalledPackage,
        latest: Option<&Release>,
        latest_error: Option<String>,
        compatible: bool,
        offline: bool,
    ) -> Self {
        let state = if !package.present {
            StatusState::Missing
        } else if offline {
            StatusState::Installed
        } else if latest_error.is_some() {
            StatusState::LatestUnavailable
        } else if latest.is_some_and(|_| !compatible) {
            StatusState::NoCompatibleAsset
        } else if latest.is_some_and(|release| release.tag_name == package.version) {
            StatusState::UpToDate
        } else if latest.is_some() {
            StatusState::UpdateAvailable
        } else {
            StatusState::Installed
        };
        Self {
            repo: package.repo.clone(),
            installed: package.version.clone(),
            latest: latest.map(|release| release.tag_name.clone()),
            state,
            path: package.display_path(),
            binaries: package.binaries,
            installed_at: package.installed_at,
            present: package.present,
            note: latest_error.map(|error| format::sanitize(&error)),
        }
    }
}

pub fn print_status(entries: &[StatusEntry], offline: bool, json: bool, painter: Painter) {
    if json {
        print_json(&entries);
        return;
    }
    if entries.is_empty() {
        println!("{}", painter.dim("no managed installs recorded"));
        return;
    }
    let width = terminal_width();
    let repo_width = 28.min(width / 3).max(16);
    let version_width = 10.min(width / 6).max(8);
    let state_width = 20.min(width / 4).max(12);
    let path_width = width
        .saturating_sub(repo_width + version_width * 2 + state_width + 12)
        .max(8);
    println!(
        "{} {} {} {} {}",
        painter.dim(&format::pad_right("REPO", repo_width)),
        painter.dim(&format::pad_right("INSTALLED", version_width)),
        painter.dim(&format::pad_right("LATEST", version_width)),
        painter.dim(&format::pad_right("STATE", state_width)),
        painter.dim("PATH")
    );
    let mut updates = 0usize;
    for entry in entries {
        if entry.state == StatusState::UpdateAvailable {
            updates += 1;
        }
        let state = format::pad_right(
            &format::truncate(entry.state.label(), state_width),
            state_width,
        );
        let state = match entry.state {
            StatusState::UpToDate => painter.good(&state),
            StatusState::UpdateAvailable => painter.warning(&state),
            StatusState::Missing => painter.error(&state),
            StatusState::NoCompatibleAsset | StatusState::LatestUnavailable => {
                painter.warning(&state)
            }
            StatusState::Installed => painter.dim(&state),
        };
        let clean_repo = format::sanitize(&entry.repo);
        let repo = format::truncate(&clean_repo, repo_width);
        let installed = format::pad_right(&format::sanitize(&entry.installed), version_width);
        let latest = entry.latest.as_deref().unwrap_or("-");
        let latest = format::pad_right(
            &format::truncate(&format::sanitize(latest), version_width),
            version_width,
        );
        let mut path = if path_width >= 8 {
            format::truncate(&format::sanitize(&entry.path), path_width).into_owned()
        } else {
            String::new()
        };
        if entry.binaries > 1 {
            path.push_str(&format!(" +{}", entry.binaries - 1));
        }
        println!(
            "{} {} {} {} {}",
            painter.bold(&format::pad_right(&repo, repo_width)),
            painter.dim(&installed),
            painter.dim(&latest),
            state,
            painter.dim(&path)
        );
        if let Some(note) = &entry.note {
            println!(
                "{} {}",
                painter.dim(&" ".repeat(repo_width + 1)),
                painter.dim(&format::truncate(
                    note,
                    width.saturating_sub(repo_width + 2)
                ))
            );
        }
    }
    println!(
        "\n{}",
        painter.dim(&format!(
            "{} managed install{} · {} update{}{}",
            entries.len(),
            if entries.len() == 1 { "" } else { "s" },
            updates,
            if updates == 1 { "" } else { "s" },
            if offline { " · offline" } else { "" }
        ))
    );
}

pub fn print_json<T: Serialize>(value: &T) {
    let mut stdout = io::stdout().lock();
    if serde_json::to_writer_pretty(&mut stdout, value).is_ok() {
        let _ = writeln!(stdout);
    }
}
