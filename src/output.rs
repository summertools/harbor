//! Human-readable non-interactive output.

use std::env;
use std::io::{self, Write};

use serde::Serialize;

use crate::Discovery;
use crate::format;
use crate::github::{Release, RepoRef};
use crate::install::{InstallOutcome, InstallPlan};
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
}

pub fn print_repos(discovery: &Discovery, show_archived: bool, json: bool, painter: Painter) {
    if json {
        let rows: Vec<RepoJson<'_>> = discovery
            .repos
            .iter()
            .filter(|repo| show_archived || !repo.archived)
            .map(|repo| RepoJson {
                source: repo.source.label(),
                full_name: &repo.full_name,
                description: repo.description.as_deref(),
                archived: repo.archived,
                fork: repo.fork,
                html_url: repo.html_url.as_deref(),
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
        let badge = painter.dim(&format!("[{}]", repo.source.label()));
        let raw_name = format::sanitize(&repo.full_name);
        let name_column = 34.min(width / 2);
        let name = format::pad_right(&format::truncate(&raw_name, name_column), name_column);
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
        let used = name_column + format::width(&repo.source.label()) + 8 + tags_width;
        let description_width = width.saturating_sub(used).max(10);
        let description = repo
            .description
            .as_deref()
            .map(format::sanitize)
            .unwrap_or_default();
        let description = format::truncate(&description, description_width);
        println!(
            "{} {}  {}{}",
            badge,
            painter.bold(&name),
            painter.dim(&description),
            tags
        );
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

pub fn print_json<T: Serialize>(value: &T) {
    let mut stdout = io::stdout().lock();
    if serde_json::to_writer_pretty(&mut stdout, value).is_ok() {
        let _ = writeln!(stdout);
    }
}
