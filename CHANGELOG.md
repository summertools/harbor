# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-11-11

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
- Interactive ratatui browser with live search, source filters, asset selection,
  progress gauge, confirmations, results, and an in-app source manager.
- Scriptable CLI with `list`, `releases`, `install`, `update`, `search`,
  `source`, `doctor`, and `config` commands; JSON output and dry runs.
- Bounded API/download sizes, GitHub host allowlisting, token isolation,
  sanitised terminal text, and no shell or automatic privilege escalation.
- English and Chinese documentation.

[0.1.0]: https://github.com/summertools/harbor/releases/tag/v0.1.0
