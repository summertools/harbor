//! Command-line interface and non-interactive workflows.

use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};

use crate::checksum;
use crate::config::{self, Config, Source};
use crate::error::{Error, Result};
use crate::format;
use crate::github::{GitHubClient, RepoRef};
use crate::install::{InstallOutcome, InstallRequest, InstallStage, Installer, Ledger};
use crate::output::{self, Painter};
use crate::platform;
use crate::ui::{self, UiOptions};

/// Safe, interactive installer for GitHub release binaries.
#[derive(Debug, Parser)]
#[command(
    name = "harbor",
    version,
    about = "Install GitHub release binaries safely and interactively",
    long_about = "Harbor discovers release binaries from GitHub organisations and repositories, \
                  chooses an asset for the current platform, verifies its SHA-256 digest, \
                  extracts it safely, and installs it atomically into a user-controlled \
                  executable directory.\n\nRun without a subcommand to open the interactive UI.",
    after_help = "SOURCES:\n  harbor source add org summertools\n  harbor source add repo BurntSushi/ripgrep\n  harbor source list\n  harbor source remove summertools\n\nTUI KEYS:\n  ↑/k, ↓/j    move          /       search\n  Tab         switch pane   s       sources\n  Enter       inspect/install  r    refresh\n  ?           help          q       quit\n\nSAFETY:\n  Only HTTPS GitHub download hosts are accepted. SHA-256 is verified when\n  published. Archives are extracted without links, devices, or path traversal.\n  Existing files are only replaced when harbor installed them or --force is used.\n\nExamples:\n  harbor\n  harbor install rtop\n  harbor install summertools/dulens --yes\n  harbor list --source summertools --all\n  harbor releases rtop\n  harbor update --yes\n  harbor doctor"
)]
pub struct Cli {
    /// Use an explicit config file.
    #[arg(long, global = true, value_name = "FILE", env = "HARBOR_CONFIG")]
    pub config: Option<PathBuf>,

    /// Install into this directory instead of the detected PATH directory.
    #[arg(long, global = true, value_name = "DIR", env = "HARBOR_BIN_DIR")]
    pub bin_dir: Option<PathBuf>,

    /// Disable ANSI colour in non-interactive output.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Suppress progress and informational lines.
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Emit machine-readable JSON where supported.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Open the interactive release browser (default).
    Tui,
    /// List repositories from configured sources.
    #[command(visible_alias = "ls")]
    List(ListArgs),
    /// List recent releases and compatible assets.
    #[command(visible_alias = "rel")]
    Releases(ReleasesArgs),
    /// Resolve, verify, and install a release.
    #[command(visible_alias = "i")]
    Install(InstallArgs),
    /// Update binaries recorded in the install ledger.
    Update(UpdateArgs),
    /// Search configured sources without installing.
    Search(SearchArgs),
    /// Manage organisation and repository sources.
    Source(SourceArgs),
    /// Check paths, permissions, token, and API connectivity.
    Doctor,
    /// Show the config path or effective config.
    Config(ConfigArgs),
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Restrict to one configured source label or name.
    #[arg(long, value_name = "SOURCE")]
    pub source: Option<String>,
    /// Include archived repositories.
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct ReleasesArgs {
    /// Repository (`owner/repo`) or a unique name from a configured org.
    pub repo: String,
    /// Show a specific tag instead of recent releases.
    #[arg(long)]
    pub tag: Option<String>,
    /// Number of releases to show.
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(usize))]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    /// Repository (`owner/repo`), or a unique name from a configured org.
    pub repo: String,
    /// Install a specific release tag.
    #[arg(long)]
    pub tag: Option<String>,
    /// Restrict asset selection with a glob (for example '*linux-musl*').
    #[arg(long)]
    pub asset: Option<String>,
    /// Select a filename or relative path inside the archive.
    #[arg(long)]
    pub binary: Option<String>,
    /// Rename the installed executable.
    #[arg(long)]
    pub name: Option<String>,
    /// Replace a file that harbor does not manage.
    #[arg(long)]
    pub force: bool,
    /// Install even when the release publishes no SHA-256 digest.
    #[arg(long)]
    pub allow_unverified: bool,
    /// Do not prompt; required when stdin or stdout is not a terminal.
    #[arg(long, short = 'y')]
    pub yes: bool,
    /// Resolve the plan and stop before downloading.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Only show what would change.
    #[arg(long)]
    pub dry_run: bool,
    /// Do not prompt.
    #[arg(long, short = 'y')]
    pub yes: bool,
    /// Replace files whose digest no longer matches the ledger.
    #[arg(long)]
    pub force: bool,
    /// Permit updates that lack a published checksum.
    #[arg(long)]
    pub allow_unverified: bool,
}

#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Text matched against repository names and descriptions.
    pub query: String,
    /// Restrict to one configured source label or name.
    #[arg(long, value_name = "SOURCE")]
    pub source: Option<String>,
    /// Include archived repositories.
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct SourceArgs {
    #[command(subcommand)]
    pub command: SourceCommand,
}

#[derive(Debug, Subcommand)]
pub enum SourceCommand {
    /// List configured sources.
    List,
    /// Add an org or repo source.
    ///
    /// Accepts `org <name>`, `repo <owner>/<repo>`, `org:<name>`,
    /// `repo:<owner>/<repo>`, or a bare org/repo.
    Add {
        #[arg(value_name = "SPEC", num_args = 1..=2)]
        spec: Vec<String>,
    },
    /// Remove a source by label or name.
    Remove { spec: String },
}

#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print the config file path.
    Path,
    /// Print the effective TOML configuration.
    Show,
}

/// Entry point used by the binary.
pub fn run(mut cli: Cli) -> Result<()> {
    let painter = Painter::new(color_enabled(cli.no_color));
    let (config, mut warnings) = Config::load(cli.config.as_deref())?;
    let config_path_for_ui = config_path(&cli);
    let command = cli.command.take().unwrap_or(Command::Tui);

    match command {
        Command::Tui => {
            if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                return Err(Error::msg(
                    "the interactive UI requires a terminal; use a subcommand such as `harbor list`",
                ));
            }
            output::print_warnings(&warnings, painter);
            let harbor = crate::Harbor::new(config, cli.bin_dir.as_deref())?;
            ui::run(
                harbor,
                UiOptions {
                    color: painter.enabled,
                    config_path: config_path_for_ui,
                },
            )
        }
        Command::List(args) => {
            output::print_warnings(&warnings, painter);
            let client = GitHubClient::new()?;
            let mut discovery = crate::discover(&config, &client);
            warnings.append(&mut discovery.warnings);
            output::print_warnings(&warnings, painter);
            if let Some(source) = &args.source {
                discovery.repos.retain(|repo| repo.source.matches(source));
            }
            output::print_repos(&discovery, args.all, cli.json, painter);
            Ok(())
        }
        Command::Releases(args) => {
            output::print_warnings(&warnings, painter);
            let client = GitHubClient::new()?;
            let repo = crate::resolve_repo(&config, &client, &args.repo)?;
            let releases = if let Some(tag) = &args.tag {
                vec![client.release_by_tag(&repo, tag)?]
            } else {
                client.list_releases(&repo, args.limit.clamp(1, 100))?
            };
            if releases.is_empty() {
                if cli.json {
                    output::print_json(&Vec::<serde_json::Value>::new());
                } else {
                    println!("{}", painter.dim("no published releases"));
                }
                return Ok(());
            }
            output::print_releases(
                &repo,
                &releases,
                platform::Platform::current(),
                cli.json,
                painter,
            );
            Ok(())
        }
        Command::Install(args) => install_command(&cli, config, warnings, args, painter),
        Command::Update(args) => update_command(&cli, config, warnings, args, painter),
        Command::Search(args) => {
            output::print_warnings(&warnings, painter);
            let client = GitHubClient::new()?;
            let mut discovery = crate::discover(&config, &client);
            warnings.append(&mut discovery.warnings);
            output::print_warnings(&warnings, painter);
            if let Some(source) = &args.source {
                discovery.repos.retain(|repo| repo.source.matches(source));
            }
            let query = args.query.to_ascii_lowercase();
            discovery.repos.retain(|repo| {
                repo.full_name.to_ascii_lowercase().contains(&query)
                    || repo.description.as_deref().is_some_and(|description| {
                        description.to_ascii_lowercase().contains(&query)
                    })
            });
            output::print_repos(&discovery, args.all, cli.json, painter);
            Ok(())
        }
        Command::Source(args) => source_command(&cli, args, painter),
        Command::Doctor => doctor_command(&cli, config, warnings, painter),
        Command::Config(args) => config_command(&cli, config, args),
    }
}

fn install_command(
    cli: &Cli,
    config: Config,
    warnings: Vec<String>,
    args: InstallArgs,
    painter: Painter,
) -> Result<()> {
    if !args.dry_run
        && !args.yes
        && (cli.json || !io::stdin().is_terminal() || !io::stdout().is_terminal())
    {
        return Err(Error::msg(
            "refusing to install without --yes because output is non-interactive",
        ));
    }
    let harbor = if args.dry_run {
        crate::Harbor::planned(config.clone(), cli.bin_dir.as_deref())?
    } else {
        crate::Harbor::new(config.clone(), cli.bin_dir.as_deref())?
    };
    output::print_warnings(&warnings, painter);
    let repo = harbor.resolve_repo(&args.repo)?;
    let mut request = InstallRequest {
        repo,
        tag: args.tag.clone(),
        asset_name: None,
        asset_glob: args.asset.clone(),
        binary: args.binary.clone(),
        name: args.name.clone(),
        force: args.force,
        allow_unverified: args.allow_unverified || config.allow_unverified,
    };
    let mut plan = harbor.plan(request.clone())?;
    if !cli.quiet && !cli.json {
        output::print_plan(&plan, painter);
    }
    if args.dry_run {
        if cli.json {
            output::print_json(&PlanJson::from(&plan));
        }
        return Ok(());
    }

    if !args.yes {
        if cli.json {
            return Err(Error::msg("--json installs require --yes"));
        }
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(Error::msg(
                "refusing to install without --yes because stdin/stdout is not a terminal",
            ));
        }
        if !plan.checksum_available() && !request.allow_unverified {
            println!();
            println!(
                "{}",
                painter.warning(
                    "This release does not publish a SHA-256 checksum. Type `unverified` to continue:"
                )
            );
            let answer = read_line()?;
            if answer.trim() != "unverified" {
                println!("{}", painter.dim("cancelled"));
                return Ok(());
            }
            request.allow_unverified = true;
            plan.request.allow_unverified = true;
        } else {
            print!("{} ", painter.bold("Install? [y/N]"));
            let _ = io::stdout().flush();
            let answer = read_line()?;
            if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                println!("{}", painter.dim("cancelled"));
                return Ok(());
            }
        }
    } else if !plan.checksum_available() && !request.allow_unverified {
        return Err(Error::ChecksumMissing(plan.choice.asset.name.clone()));
    }

    let mut progress = ConsoleProgress::new(!cli.quiet);
    let outcome = harbor
        .installer()
        .install(&plan, |event| progress.update(event))?;
    progress.finish();
    if cli.json {
        output::print_json(&OutcomeJson::from(&outcome));
    } else {
        output::print_outcome(&outcome, painter);
    }
    Ok(())
}

fn update_command(
    cli: &Cli,
    config: Config,
    warnings: Vec<String>,
    args: UpdateArgs,
    painter: Painter,
) -> Result<()> {
    if !args.dry_run && !args.yes && (cli.json || !io::stdin().is_terminal()) {
        return Err(Error::msg(
            "refusing to update without --yes because input is non-interactive",
        ));
    }
    output::print_warnings(&warnings, painter);
    let client = GitHubClient::new()?;
    let ledger = Ledger::load()?;
    if ledger.entries.is_empty() {
        if cli.json {
            output::print_json(&Vec::<OutcomeJson>::new());
        } else if !cli.quiet {
            println!(
                "{}",
                painter.dim("no managed installs recorded; nothing to update")
            );
        }
        return Ok(());
    }
    let mut outcomes: Vec<OutcomeJson> = Vec::new();
    for (path, entry) in &ledger.entries {
        let repo = RepoRef::parse(&entry.repo)?;
        let target = Path::new(path);
        let name = target
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| Error::msg(format!("invalid ledger target path {path}")))?;
        let bin_path = PathBuf::from(&entry.bin_dir);
        let bin_dir = if args.dry_run {
            platform::planned_bin_dir(Some(&bin_path), None)?
        } else {
            platform::resolve_bin_dir(Some(&bin_path), None)?
        };
        let installer = Installer::new(&client, &config, bin_dir);
        let request = InstallRequest {
            repo: repo.clone(),
            tag: None,
            asset_name: None,
            asset_glob: None,
            binary: None,
            name: Some(name.to_owned()),
            force: args.force,
            allow_unverified: args.allow_unverified || config.allow_unverified,
        };
        let plan = installer.plan(request)?;
        let current_ok = fs::symlink_metadata(target).is_ok_and(|metadata| metadata.is_file())
            && checksum::sha256_file(target)
                .ok()
                .is_some_and(|digest| digest == entry.binary_sha256);
        if plan.release.tag_name == entry.version && current_ok {
            if cli.json {
                outcomes.push(OutcomeJson {
                    repo: repo.full_name(),
                    version: entry.version.clone(),
                    path: target.display().to_string(),
                    asset: entry.asset.clone(),
                    sha256: entry.binary_sha256.clone(),
                    bytes: 0,
                    checksum_verified: true,
                    already_current: true,
                    warning: None,
                });
            } else if !cli.quiet {
                println!(
                    "{} {} is up to date ({})",
                    painter.good("✓"),
                    painter.bold(&format::sanitize(&repo.full_name())),
                    entry.version
                );
            }
            continue;
        }
        if !cli.quiet && !cli.json {
            println!(
                "{} {} {} → {}",
                painter.accent("update"),
                painter.bold(&format::sanitize(&repo.full_name())),
                entry.version,
                plan.release.tag_name
            );
        }
        if args.dry_run {
            continue;
        }
        if !args.yes {
            if !io::stdin().is_terminal() {
                return Err(Error::msg(
                    "refusing to update without --yes because stdin is not a terminal",
                ));
            }
            print!(
                "{} ",
                painter.bold(&format!("Update {}? [y/N]", repo.full_name()))
            );
            let _ = io::stdout().flush();
            let answer = read_line()?;
            if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                println!("{}", painter.dim("skipped"));
                continue;
            }
        }
        let mut progress = ConsoleProgress::new(!cli.quiet);
        let outcome = installer.install(&plan, |event| progress.update(event))?;
        progress.finish();
        if !cli.quiet && !cli.json {
            output::print_outcome(&outcome, painter);
        }
        outcomes.push(OutcomeJson::from(&outcome));
    }
    if cli.json {
        output::print_json(&outcomes);
    }
    Ok(())
}

fn source_command(cli: &Cli, args: SourceArgs, painter: Painter) -> Result<()> {
    let path = config_path(cli);
    let mut config = if path.exists() {
        let (config, warnings) = Config::load(Some(&path))?;
        output::print_warnings(&warnings, painter);
        config
    } else {
        Config::default()
    };
    match args.command {
        SourceCommand::List => {
            if cli.json {
                output::print_json(&config.sources);
            } else {
                for source in &config.sources {
                    println!(
                        "{} {}",
                        painter.dim(&format!("[{}]", source.kind.label())),
                        source.name
                    );
                }
                if config.sources.is_empty() {
                    println!("{}", painter.dim("no sources configured"));
                }
            }
        }
        SourceCommand::Add { spec } => {
            let source = match spec.as_slice() {
                [value] => Source::parse_spec(value)?,
                [kind, value] if kind.eq_ignore_ascii_case("org") => Source::org(value.as_str())?,
                [kind, value] if kind.eq_ignore_ascii_case("repo") => Source::repo(value.as_str())?,
                _ => {
                    return Err(Error::Source(
                        "use `source add org <name>` or `source add repo <owner>/<repo>`"
                            .to_owned(),
                    ));
                }
            };
            if config.find_source(&source.label()).is_some() {
                if cli.json {
                    output::print_json(&config.sources);
                } else {
                    println!(
                        "{} {} is already configured",
                        painter.dim("·"),
                        painter.bold(&source.label())
                    );
                }
                return Ok(());
            }
            let label = config.add_source(source)?;
            config.save_to(&path)?;
            if cli.json {
                output::print_json(&config.sources);
            } else {
                println!(
                    "{} added {} to {}",
                    painter.good("✓"),
                    painter.bold(&label),
                    config::display_config_path(&path)
                );
            }
        }
        SourceCommand::Remove { spec } => {
            if !config.remove_source(&spec) {
                return Err(Error::Source(format!(
                    "source {:?} is not configured",
                    format::sanitize(&spec)
                )));
            }
            config.save_to(&path)?;
            if cli.json {
                output::print_json(&config.sources);
            } else {
                println!("{} removed {}", painter.good("✓"), format::sanitize(&spec));
            }
        }
    }
    Ok(())
}

fn config_command(cli: &Cli, config: Config, args: ConfigArgs) -> Result<()> {
    let path = config_path(cli);
    match args.command {
        ConfigCommand::Path => println!("{}", format::sanitize(&path.display().to_string())),
        ConfigCommand::Show => {
            let text = toml::to_string_pretty(&config)
                .map_err(|error| Error::Config(format!("cannot serialise config: {error}")))?;
            print!("{text}");
        }
    }
    Ok(())
}

fn doctor_command(
    cli: &Cli,
    config: Config,
    warnings: Vec<String>,
    painter: Painter,
) -> Result<()> {
    output::print_warnings(&warnings, painter);
    let harbor = crate::Harbor::new(config.clone(), cli.bin_dir.as_deref())?;
    let config_path = config_path(cli);
    let token = crate::github::token_from_env().is_some();
    let mut checks: Vec<Check> = Vec::new();
    checks.push(Check::ok(
        "platform",
        format!("{} · {}", harbor.platform.label(), std::env::consts::OS),
    ));
    checks.push(Check::ok(
        "config",
        format!(
            "{} ({} sources)",
            config::display_config_path(&config_path),
            config.sources.len()
        ),
    ));
    checks.push(Check::ok(
        "install dir",
        format!(
            "{} ({})",
            platform::compact_path(&harbor.bin_dir.path),
            if harbor.bin_dir.in_path {
                "on PATH"
            } else {
                "not on PATH"
            }
        ),
    ));
    checks.push(Check::ok(
        "token",
        if token {
            "GH_TOKEN/GITHUB_TOKEN is set"
        } else {
            "not set (public API rate limits apply)"
        },
    ));
    checks.push(Check::ok(
        "ledger",
        platform::compact_path(&Ledger::default_path()),
    ));
    match harbor.client.rate_limit() {
        Ok((limit, remaining, reset)) => {
            checks.push(Check::ok(
                "github api",
                format!(
                    "{remaining}/{limit} core requests remaining ({})",
                    reset_hint(reset)
                ),
            ));
        }
        Err(error) => checks.push(Check::warn("github api", error.to_string())),
    }
    if cli.json {
        output::print_json(&checks);
    } else {
        for check in &checks {
            let symbol = match check.level {
                Level::Ok => painter.good("✓"),
                Level::Warn => painter.warning("!"),
            };
            println!(
                "{} {:<12} {}",
                symbol,
                check.name,
                format::sanitize(&check.detail)
            );
        }
    }
    Ok(())
}

fn reset_hint(reset: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let seconds = reset.saturating_sub(now);
    if seconds == 0 {
        "resets now".to_owned()
    } else if seconds < 3600 {
        format!("resets in {} min", seconds.div_ceil(60))
    } else {
        format!("resets in {} h", seconds.div_ceil(3600))
    }
}

fn config_path(cli: &Cli) -> PathBuf {
    cli.config
        .clone()
        .or_else(|| {
            env::var_os("HARBOR_CONFIG")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(config::default_config_path)
}

fn read_line() -> Result<String> {
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line)
}

fn color_enabled(no_color: bool) -> bool {
    !no_color && env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal()
}

struct ConsoleProgress {
    enabled: bool,
    tty: bool,
    stage: Option<InstallStage>,
    line_active: bool,
    last_update: std::time::Instant,
}

impl ConsoleProgress {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            tty: io::stderr().is_terminal(),
            stage: None,
            line_active: false,
            last_update: std::time::Instant::now(),
        }
    }

    fn update(&mut self, event: crate::install::ProgressEvent) {
        if !self.enabled {
            return;
        }
        if self.stage != Some(event.stage) {
            self.finish_line();
            self.stage = Some(event.stage);
            if self.tty {
                eprint!("{} {}", console_spinner(event.stage), event.stage.label());
                self.line_active = true;
            } else {
                eprintln!("{}", event.stage.label());
            }
        }
        if matches!(
            event.stage,
            InstallStage::Downloading | InstallStage::Checksum
        ) && self.tty
        {
            if self.last_update.elapsed().as_millis() < 80
                && event.total.is_none_or(|total| event.current < total)
            {
                return;
            }
            self.last_update = std::time::Instant::now();
            let suffix = match event.total.filter(|total| *total > 0) {
                Some(total) => {
                    let ratio = (event.current as f64 / total as f64).clamp(0.0, 1.0);
                    format!(
                        "  {:>3.0}%  {}/{}",
                        ratio * 100.0,
                        crate::format::bytes(event.current),
                        crate::format::bytes(total)
                    )
                }
                None => format!("  {}", crate::format::bytes(event.current)),
            };
            eprint!(
                "\r\x1b[2K{} {}{}",
                console_spinner(event.stage),
                event.stage.label(),
                suffix
            );
            self.line_active = true;
        }
    }

    fn finish_line(&mut self) {
        if self.line_active {
            eprintln!();
            self.line_active = false;
        }
    }

    fn finish(&mut self) {
        self.finish_line();
    }
}

fn console_spinner(stage: InstallStage) -> &'static str {
    match stage {
        InstallStage::Checksum => "◌",
        InstallStage::Downloading => "↓",
        InstallStage::Verifying => "✓",
        InstallStage::Extracting => "◇",
        InstallStage::Installing => "→",
        InstallStage::Complete => "✓",
    }
}

#[derive(serde::Serialize)]
struct OutcomeJson {
    repo: String,
    version: String,
    path: String,
    asset: String,
    sha256: String,
    bytes: u64,
    checksum_verified: bool,
    already_current: bool,
    warning: Option<String>,
}

impl From<&InstallOutcome> for OutcomeJson {
    fn from(outcome: &InstallOutcome) -> Self {
        Self {
            repo: outcome.repo.clone(),
            version: outcome.version.clone(),
            path: outcome.path.display().to_string(),
            asset: outcome.asset.clone(),
            sha256: outcome.sha256.clone(),
            bytes: outcome.bytes,
            checksum_verified: outcome.checksum_verified,
            already_current: outcome.already_current,
            warning: outcome.warning.clone(),
        }
    }
}

#[derive(serde::Serialize)]
struct PlanJson {
    repo: String,
    version: String,
    asset: String,
    checksum_available: bool,
    bin_dir: String,
}

impl From<&crate::install::InstallPlan> for PlanJson {
    fn from(plan: &crate::install::InstallPlan) -> Self {
        Self {
            repo: plan.request.repo.full_name(),
            version: plan.release.tag_name.clone(),
            asset: plan.choice.asset.name.clone(),
            checksum_available: plan.checksum_available(),
            bin_dir: plan.bin_dir.display().to_string(),
        }
    }
}

#[derive(serde::Serialize)]
struct Check {
    level: Level,
    name: String,
    detail: String,
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum Level {
    Ok,
    Warn,
}

impl Check {
    fn ok(name: &str, detail: impl Into<String>) -> Self {
        Self {
            level: Level::Ok,
            name: name.to_owned(),
            detail: detail.into(),
        }
    }

    fn warn(name: &str, detail: impl Into<String>) -> Self {
        Self {
            level: Level::Warn,
            name: name.to_owned(),
            detail: detail.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_source_and_install_commands() {
        let cli = Cli::try_parse_from(["harbor", "install", "rtop", "--asset", "*musl*", "--yes"])
            .unwrap();
        assert!(matches!(cli.command, Some(Command::Install(_))));
        let cli = Cli::try_parse_from(["harbor", "source", "add", "repo:owner/tool"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Source(_))));
    }

    #[test]
    fn release_helpers_do_not_need_a_network() {
        assert_eq!(crate::release::AssetKind::TarGz.label(), "tar.gz");
    }
}
