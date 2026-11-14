# harbor

[![CI](https://github.com/summertools/harbor/actions/workflows/ci.yml/badge.svg)](https://github.com/summertools/harbor/actions/workflows/ci.yml)


**A safe, interactive installer for GitHub release binaries.**

English | [简体中文](README.zh-CN.md)

`harbor` discovers release assets from GitHub organisations and repositories,
picks the asset that matches the current operating system and CPU, verifies its
SHA-256 digest, extracts it without trusting archive paths, and installs the
executable atomically into a user-controlled directory on `PATH`.

The default source is **`summertools`**, but any GitHub organisation or
repository can be added. The same library powers the polished terminal UI and
the scriptable CLI.

```text
  harbor  safe release installer · GitHub releases                  linux-aarch64-gnu
  bin ~/.local/bin  on PATH                    2 installed · 1 update · 3 not installed

╭─ repositories · 5/5 · all · all sources ─────────────────╮╭─ details ───────────────────────────────╮
│› ● summertools/dulens    org:summertools  ✓ v0.2.0        ││status   installed v0.2.0 · up to date    │
│  ● summertools/harbor    org:summertools  ○               ││path     ~/.local/bin/dulens              │
│  ● summertools/mdtui     org:summertools  ↑ v0.0.1        ││installed 2026-11-11 10:26 UTC            │
│  ● summertools/rscreen   org:summertools  ○               ││release v0.2.0  2026-11-11                │
│  ● summertools/rtop      org:summertools  ○               ││checksum SHA-256 published  1021 KiB      │
│                                                            ││assets · 1 compatible · 3 other           │
╰────────────────────────────────────────────────────────────╯╰─────────────────────────────────────────╯
 ↑↓ move  Enter release  i install  Tab assets  / search  v status  f source  s sources  r refresh  ? help  q quit
```

## Features

- **Default summertools source** with automatic repository discovery.
- **Custom sources**: add any GitHub organisation or a single `owner/repo`.
- **Platform-aware asset matching** for Linux, macOS, and Windows across
  x86_64, aarch64, x86, ARM, and RISC-V naming conventions.
- **SHA-256 verification** when the release publishes checksums, with a clear
  explicit opt-in for unverified assets.
- **Safe extraction** for `.tar.gz`, `.tgz`, `.tar`, `.zip`, and raw binaries:
  absolute paths, `..`, symlinks, hard links, devices, and FIFOs are rejected.
- **Atomic installation** with mode `0755`, a same-directory staging file, and
  rollback if a replacement fails.
- **Install ledger and status at a glance**: every repository row shows
  `✓ installed vX`, `↑ update vX → vY`, `! missing`, `! no compatible asset`, or
  `○ not installed`. The header summarises installed, update, and not-installed
  counts. Unmanaged files require `--force` or the explicit overwrite toggle.
- **Polished ratatui interface** with search, source filters, asset selection,
  live progress, confirmations, and a source manager.
- **Scriptable CLI** with JSON output, dry runs, explicit tags, asset globs,
  archive-internal binary selection, and non-interactive `--yes`.
- **No shell execution**, no automatic `sudo`, no automatic PATH edits, and no
  background services.
- Bounded API responses and downloads, sanitised terminal text, and GitHub-host
  allowlisting.

## Installation

From this repository:

```bash
cargo install --path .
```

The binary is named `harbor`. Rust 1.88 or newer is required.

## Quick start

```bash
# Open the interactive browser (default source: summertools)
harbor

# Install a repository from the default organisation
harbor install dulens

# Install directly from any GitHub repository
harbor install BurntSushi/ripgrep --yes

# See installed versions, update status, and installed paths
harbor status
harbor status --offline

# Add a custom organisation or repository to the saved sources
harbor source add org rust-lang
harbor source add repo BurntSushi/ripgrep

# Inspect before installing
harbor list
harbor list --installed
harbor list --available
harbor releases rtop
harbor install rtop --dry-run
```

If a repository exists under more than one configured organisation, use the
full `owner/repo` form. A bare name is accepted only when it is unique.

## Interactive UI

Run `harbor` with no subcommand.

| Key | Action |
| --- | --- |
| `↑` / `k`, `↓` / `j` | Move through repositories or assets |
| `PgUp` / `PgDn`, `Home` / `End` | Page, first, last |
| `Enter` | Load the selected release, then open the asset list / confirm |
| `Tab` | Switch between the repository and asset panes |
| `i` | Install the selected asset |
| `/` | Search repositories live |
| `Enter` / `Esc` | Apply / cancel search |
| `Ctrl+U` | Clear the search text |
| `v` | Cycle status filter: all / installed / updates / not installed |
| `f` | Cycle the source filter |
| `s` | Manage sources |
| `r` | Refresh repositories |
| `?` | Help |
| `q` | Quit |

In the confirmation screen:

| Key | Action |
| --- | --- |
| `Enter` / `y` | Install |
| `u` | Allow an unverified install when no checksum is published |
| `o` | Allow replacing an unmanaged file |
| `Esc` / `n` | Cancel |

### Installed status

Every repository row carries a compact status chip:

| Chip | Meaning |
| --- | --- |
| `✓ vX` | Installed and, when latest metadata is available, up to date |
| `↑ vX → vY` | Installed version differs from the latest release |
| `! missing vX` | The ledger says installed, but the executable is gone |
| `! no compatible asset` | The latest release has no asset for this platform |
| `○ not installed` | No managed install is recorded |
| `○ checking…` | Latest release metadata is being fetched |

The header shows `N installed · N updates · N not installed`. Press `v` to cycle the
list filter between all repositories, installed ones, updates, and not-installed
ones. The details pane repeats the installed version, executable path, install
time, update state, and any platform-incompatibility explanation.

If a release exists but only publishes assets for other systems, Harbor keeps
the release visible, lists those assets with reasons such as *different
architecture* or *different operating system*, and disables installation instead
of reporting a misleading load failure. Repositories with no usable release show
a distinct **no installable release** state.

The source manager accepts `org:name`, `repo:owner/name`, `org name`,
`repo owner/name`, or a bare `owner/repo`. Changes are written atomically to the
config file.

## CLI

```text
harbor [--config FILE] [--bin-dir DIR] [--no-color] [--quiet] [--json] [COMMAND]
```

| Command | Purpose |
| --- | --- |
| `harbor` / `harbor tui` | Interactive browser |
| `harbor list [--source S] [--all] [--installed\|--available]` | List repositories and their install status |
| `harbor status [--offline]` | Show installed versions, latest versions, update state, and paths |
| `harbor releases <repo> [--tag T] [--limit N]` | List releases and compatible assets |
| `harbor install <repo>` | Resolve, verify, and install |
| `harbor update` | Update managed installs from the ledger |
| `harbor search <query>` | Search configured repositories |
| `harbor source add\|list\|remove` | Manage sources |
| `harbor doctor` | Check paths, permissions, token, and API connectivity |
| `harbor config path\|show` | Show the config location or effective config |

### Install options

```bash
harbor install owner/repo \
  --tag v1.2.3 \
  --asset '*linux-musl*' \
  --binary bin/tool \
  --name tool \
  --bin-dir ~/.local/bin \
  --yes
```

| Option | Meaning |
| --- | --- |
| `--tag <TAG>` | Install a specific release instead of the latest stable one |
| `--asset <GLOB>` | Restrict compatible assets by name |
| `--binary <NAME>` | Choose a filename or relative path inside the archive |
| `--name <NAME>` | Rename the installed executable |
| `--bin-dir <DIR>` | Override the detected executable directory |
| `--force` | Replace a file that harbor does not manage |
| `--allow-unverified` | Permit an asset with no usable SHA-256 digest |
| `--yes` | Non-interactive confirmation (required without a TTY) |
| `--dry-run` | Resolve and print the plan without downloading |

With `--json`, `install` and `update` emit a machine-readable result on stdout;
progress stays on stderr. `list`, `status`, `releases`, `source list`, and
`doctor` also support `--json`. `harbor status --offline` works without network
access and reports the ledger only.

## Sources and configuration

The default config path is:

```text
~/.config/harbor/config.toml
```

`$XDG_CONFIG_HOME/harbor/config.toml`, `--config <FILE>`, and `HARBOR_CONFIG`
are also supported. Default sources are used when the file does not exist yet.

```toml
bin_dir = "~/.local/bin"
allow_unverified = false
include_prereleases = false
max_download_mb = 512

[[sources]]
type = "org"
name = "summertools"

[[sources]]
type = "repo"
name = "BurntSushi/ripgrep"
```

- `bin_dir` is optional; the CLI flag and `HARBOR_BIN_DIR` override it.
- `allow_unverified = true` is a persistent version of `--allow-unverified`.
  Leave it off unless you trust the source.
- `include_prereleases = true` lets the latest endpoint fall back to a
  prerelease when no stable release exists.
- `max_download_mb` caps each downloaded asset (1–4096 MiB).
- Sources are limited to 64 entries. Invalid or duplicate sources are ignored
  with a warning when loading.

Add/remove commands update the file atomically with mode `0600`. Harbor never
edits shell startup files automatically.

## Installation directory and PATH

Resolution order:

1. `--bin-dir <DIR>`
2. `bin_dir` in the config
3. `HARBOR_BIN_DIR`
4. The first writable user directory already on `PATH` among
   `~/.local/bin`, `~/bin`, `~/.cargo/bin`, and the platform data directory
5. `~/.local/bin` (created if needed)

If the chosen directory is not on `PATH`, Harbor prints a warning and the
install still succeeds. Add the directory to your shell configuration yourself;
Harbor deliberately does not modify it.

Harbor never invokes `sudo`. If a system directory is not writable, choose a
user directory or run the command with appropriate permissions yourself.

## Asset selection

Matching uses lower-case tokens from the asset name:

- OS tokens: `linux`, `darwin` / `macos` / `osx` / `apple`, `windows` / `win32`
  / `win64`.
- Architecture tokens: `x86_64` / `amd64` / `x64`, `aarch64` / `arm64`,
  `i686` / `i386`, `armv7` / `armhf`, `riscv64`.
- Linux prefers static `musl` builds when both `musl` and `gnu` are available.
- Checksums, signatures, SBOMs, source archives, installers (`.deb`, `.rpm`,
  `.msi`, `.dmg`, `.pkg`), and unrelated metadata are excluded.
- Ties are reported as ambiguous. Resolve them with `--asset` or the asset list
  in the interactive UI.

Archives are searched for an executable whose name matches the repository, or
for the only executable with a sensible executable bit/name. Use `--binary`
when several utilities are shipped together.

## Security model

Harbor is designed for a hostile network and a shared terminal:

- **HTTPS only.** The GitHub API host is fixed. Asset URLs and every manual
  redirect are checked against `github.com`, `api.github.com`,
  `*.githubusercontent.com`, and the known GitHub release-asset host.
- **Token hygiene.** `GH_TOKEN` or `GITHUB_TOKEN` is sent only to
  `api.github.com` and `github.com`, never to object/S3 hosts and never logged.
- **Verification before installation.** If the release publishes a checksum,
  the downloaded archive is hashed and compared before extraction. Missing or
  unusable checksums require an explicit opt-in (`--allow-unverified`, the `u`
  key in the confirm screen, or `allow_unverified = true`).
- **Path-safe extraction.** Archive entries with absolute paths, `..`,
  symlinks, hard links, devices, or FIFOs are rejected. File count and expanded
  size are capped. Extraction happens in a private temporary directory.
- **Atomic replacement.** The new executable is written to a same-directory
  staging file, synced, chmodded, and renamed. A failed replacement restores the
  previous file.
- **Overwrite protection.** A ledger records the repository, version, asset
  digest, installed binary digest, and directory. Harbor only replaces a file
  whose current digest matches the ledger; anything else needs `--force`.
- **Bounded input.** API responses, checksum files, asset downloads, archive
  entries, and search input are all limited. Terminal output is
  sanitised against ANSI and bidi control characters.
- **No shell execution.** Harbor does not call `sh`, `sudo`, `curl`, or the
  installed program. The network, filesystem, and terminal are the only
  interfaces.

The ledger lives at:

```text
~/.local/state/harbor/installs.json
```

or `$XDG_STATE_HOME/harbor/installs.json` on Unix, and under
`%LOCALAPPDATA%\harbor\state` on Windows. It is written with mode `0600`.

### Threat-model boundaries

Harbor cannot make a malicious upstream trustworthy. A repository owner can
publish malicious binaries and matching checksums. Use trusted sources and
review release notes. Harbor also does not implement GPG/sigstore verification;
it implements SHA-256 integrity checking for the asset selected from the GitHub
release metadata.

## Update behavior

```bash
harbor update --dry-run
harbor update --yes
```

For each ledger entry Harbor fetches the latest stable release. It skips the
entry when the version and installed digest still match. If the managed file is
missing, it is reinstalled. If the file was modified outside Harbor, the update
stops and asks for `--force` rather than silently replacing it.

## Library usage

The binary is a thin consumer of the public library modules.

```rust
use harbor::{Config, Harbor, InstallRequest, RepoRef};

fn main() -> harbor::Result<()> {
    let (config, _warnings) = Config::load(None)?;
    let harbor = Harbor::new(config, None)?;
    let request = InstallRequest {
        repo: RepoRef::parse("summertools/dulens")?,
        tag: None,
        asset_name: None,
        asset_glob: None,
        binary: None,
        name: None,
        force: false,
        allow_unverified: false,
    };
    let outcome = harbor.install(request, |_progress| {})?;
    println!("installed to {}", outcome.path.display());
    Ok(())
}
```

Useful modules:

| Module | Responsibility |
| --- | --- |
| `config` | Sources, safety settings, atomic config I/O |
| `github` | GitHub REST client and models |
| `release` | Platform asset scoring and compatibility |
| `checksum` | Checksum asset discovery, parsing, SHA-256 |
| `download` | Host-restricted, bounded downloads |
| `archive` | Safe extraction and executable discovery |
| `install` | Plans, locking, ledger, atomic install |
| `platform` | OS/arch detection and bin-directory resolution |
| `ui` | Interactive ratatui browser |

## Platform support

| Capability | Linux | macOS | Windows |
| --- | :---: | :---: | :---: |
| Asset matching | Yes | Yes | Yes |
| SHA-256 verification | Yes | Yes | Yes |
| `.tar.gz`, `.tgz`, `.tar` | Yes | Yes | Yes |
| `.zip` | Yes | Yes | Yes |
| Raw binary | Yes | Yes | Yes |
| Atomic replacement | Yes | Yes | Best effort |
| Ledger / update | Yes | Yes | Yes |
| Interactive UI | Yes | Yes | Yes (Windows Terminal recommended) |
| Config location | XDG | XDG-style | `%APPDATA%` |

Windows replacement uses a backup-and-rename fallback because the platform does
not guarantee POSIX-style atomic rename over an existing file.

## Limitations

- GitHub.com only; GitHub Enterprise and GitLab sources are not supported yet.
- Supported containers are `.tar.gz` / `.tgz` / `.tar` / `.zip` / raw binaries.
  `.tar.xz`, `.tar.zst`, `.deb`, `.rpm`, `.msi`, `.dmg`, and `.pkg` are skipped.
- Signature verification beyond SHA-256 is not implemented.
- Private repositories need a token with access; download tokens are only sent
  to GitHub API/web hosts.
- Harbor does not install system packages, services, completions, or manual
  pages; it installs one executable.
- No automatic PATH mutation or privilege escalation.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo audit
cargo build --release
```

The tests cover asset scoring across platforms, checksum parsing, archive path
traversal and link rejection, raw-text asset rejection, atomic install guards,
ledger round-trips, config validation, TUI layout at common sizes, and terminal
safety.

## License

MIT. See [LICENSE](LICENSE).
