# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-11-15

### Added

- GitHub Release discovery for the default `summertools` organisation and
  custom organisation or repository sources.
- Platform-aware asset scoring for Linux, macOS, and Windows across x86_64,
  aarch64, x86, ARM, and RISC-V naming conventions.
- SHA-256 checksum discovery and verification, with explicit opt-in for
  releases that publish no usable digest.
- Safe `.tar.gz`, `.tgz`, `.tar`, `.zip`, and raw-binary handling with rejection
  of traversal paths, links, devices, FIFOs, and text masquerading as binaries.
- Atomic installs into a detected or explicit user executable directory, with
  mode `0755`, staged replacement, and rollback.
- Install ledger with digest-based overwrite protection and `harbor update`.
- Installed/available visibility in the TUI: right-aligned status chips for
  installed versions, updates, missing files, incompatible releases, and
  not-installed repositories.
- Header counters for installed, update, and available repositories, plus a `v`
  status filter cycling all / installed / updates / not installed.
- Detail-pane install status with recorded version, executable path, install
  time, and a clear explanation when a release has no compatible asset.
- Synthetic repository rows for ledger installs whose source is no longer
  configured, keeping all managed binaries visible and updateable.
- `harbor status [--offline]` with latest-version comparison, update state,
  missing-file detection, JSON output, and no-network mode.
- `harbor list --installed` and `--available`, and an install-status column in
  `harbor list` output.
- Opt-in automatic updates via `auto_update = true`: safe SemVer upgrades only,
  compatible assets, mandatory SHA-256 verification, and no unmanaged-file
  replacement. Install-lock ledger revalidation prevents concurrent runs from
  overwriting or downgrading a newer install. The UI shows progress in the
  footer; `harbor autoupdate` supports cron/timer usage and
  `--check`/`--json` reporting.
- Interactive ratatui browser with live search, source filters, asset selection,
  progress gauge, confirmations, results, and an in-app source manager.
- Scriptable CLI with `list`, `releases`, `install`, `update`, `search`,
  `source`, `doctor`, and `config` commands; JSON output and dry runs.
- Bounded API/download sizes, GitHub host allowlisting, token isolation,
  sanitised terminal text, and no shell or automatic privilege escalation.
- English and Chinese documentation.

### Fixed

- A release with assets for other operating systems or architectures is no
  longer reported as "could not load release"; Harbor now shows the release,
  lists incompatible assets with reasons, and disables installation.
- Repositories with no usable release get a distinct "no installable release"
  state instead of a generic load error.
- Prefetching latest metadata for installed repositories no longer races with
  the selected-repository request or clears the wrong loading state.

[Unreleased]: https://github.com/summertools/harbor/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/summertools/harbor/releases/tag/v0.1.0
