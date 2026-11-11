# Security

## Reporting

Please report security issues privately through the repository's GitHub
Security Advisory page rather than a public issue.

## Design notes

Harbor downloads public GitHub release assets. Its trust boundaries are:

- GitHub API metadata is fetched over HTTPS from a fixed host.
- Asset URLs and manual redirects are restricted to known GitHub download
  hosts.
- A published SHA-256 digest is required by default. Unverified installation
  requires an explicit flag or confirmation.
- Archives are not trusted: absolute paths, `..`, links, devices, and FIFOs are
  rejected, and extraction is bounded.
- Existing executables are replaced only when the ledger digest matches; other
  files require `--force`.
- Tokens from `GH_TOKEN`/`GITHUB_TOKEN` are never sent to object-storage hosts
  and are never printed.

Harbor cannot validate the intent of an upstream maintainer. A malicious
release with a matching checksum is still malicious. Use trusted sources.
