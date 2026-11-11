//! SHA-256 checksum discovery, parsing, and verification.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::github::{Asset, Release};

/// One digest line from a checksum file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChecksumEntry {
    pub sha256: String,
    pub file: Option<String>,
}

/// True for files that commonly contain SHA-256 digests.
#[must_use]
pub fn is_checksum_asset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".sha256")
        || lower.ends_with(".sha256sum")
        || lower.ends_with(".sha256.txt")
        || lower.ends_with("sha256sums")
        || lower.ends_with("sha256sums.txt")
        || lower.ends_with("checksums.txt")
        || lower.ends_with("checksums.sha256")
        || lower.ends_with("checksums")
}

/// Checksum assets that may describe `selected`, best candidate first.
#[must_use]
pub fn checksum_assets<'a>(release: &'a Release, selected: &Asset) -> Vec<&'a Asset> {
    let mut assets: Vec<&Asset> = release
        .assets
        .iter()
        .filter(|asset| is_checksum_asset(&asset.name))
        .collect();
    assets.sort_by_key(|asset| {
        let lower = asset.name.to_ascii_lowercase();
        let selected = selected.name.to_ascii_lowercase();
        if lower == format!("{selected}.sha256") {
            0
        } else if lower == format!("{selected}.sha256sum") {
            1
        } else if lower.starts_with(&selected) {
            2
        } else if lower.contains("checksums") || lower.contains("sha256sums") {
            3
        } else {
            4
        }
    });
    assets.truncate(8);
    assets
}

/// Parse the common checksum file formats:
///
/// ```text
/// <64 hex>  file.tar.gz
/// <64 hex> *file.tar.gz
/// SHA256 (file.tar.gz) = <64 hex>
/// <64 hex>
/// ```
#[must_use]
pub fn parse_checksums(text: &str) -> Vec<ChecksumEntry> {
    let mut entries = Vec::new();
    for line in text.lines().take(4096) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(entry) = parse_line(line)
            && !entries.iter().any(|existing| existing == &entry)
        {
            entries.push(entry);
        }
    }
    entries
}

fn parse_line(line: &str) -> Option<ChecksumEntry> {
    if let Some(rest) = line.strip_prefix("SHA256 (")
        && let Some((file, digest)) = rest.split_once(") = ")
    {
        let digest = normalize_digest(digest.trim())?;
        return Some(ChecksumEntry {
            sha256: digest,
            file: Some(file.trim().to_owned()),
        });
    }
    let mut parts = line.split_whitespace();
    let digest = normalize_digest(parts.next()?)?;
    let file = parts
        .next()
        .map(|name| name.strip_prefix('*').unwrap_or(name).trim().to_owned());
    Some(ChecksumEntry {
        sha256: digest,
        file: file.filter(|file| !file.is_empty()),
    })
}

fn normalize_digest(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    if value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Some(value)
    } else {
        None
    }
}

/// Choose the digest that belongs to `asset_name`.
///
/// A digest without a filename is accepted only when the file contains exactly
/// one unique hash. Ambiguous multi-hash files are rejected rather than risking
/// verification against the wrong artifact.
pub fn expected_digest(entries: &[ChecksumEntry], asset_name: &str) -> Result<Option<String>> {
    let asset_lower = asset_name.to_ascii_lowercase();
    let asset_base = asset_lower.rsplit('/').next().unwrap_or(&asset_lower);
    let mut matches: Vec<String> = entries
        .iter()
        .filter(|entry| {
            entry.file.as_ref().is_some_and(|file| {
                let file = file.to_ascii_lowercase();
                file == asset_lower || file.rsplit('/').next() == Some(asset_base)
            })
        })
        .map(|entry| entry.sha256.clone())
        .collect();
    matches.sort();
    matches.dedup();
    if matches.len() > 1 {
        return Err(Error::ChecksumInvalid(format!(
            "multiple different SHA-256 entries match {asset_name}"
        )));
    }
    if let Some(digest) = matches.pop() {
        return Ok(Some(digest));
    }

    let mut anonymous: Vec<String> = entries
        .iter()
        .filter(|entry| entry.file.is_none())
        .map(|entry| entry.sha256.clone())
        .collect();
    anonymous.sort();
    anonymous.dedup();
    match anonymous.len() {
        0 => Ok(None),
        1 => Ok(anonymous.pop()),
        _ => Err(Error::ChecksumInvalid(
            "checksum file contains multiple hashes without filenames".to_owned(),
        )),
    }
}

/// Compute the SHA-256 digest of a file without loading it into memory.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Compare a computed digest with an expected one.
pub fn verify_digest(actual: &str, expected: &str) -> bool {
    actual.eq_ignore_ascii_case(expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_common_formats() {
        let digest = "a".repeat(64);
        let text = format!(
            "{digest}  tool.tar.gz\n{digest} *other.zip\nSHA256 (third.tar.gz) = {digest}\n{digest}\n"
        );
        let entries = parse_checksums(&text);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].file.as_deref(), Some("tool.tar.gz"));
        assert_eq!(entries[1].file.as_deref(), Some("other.zip"));
        assert_eq!(entries[2].file.as_deref(), Some("third.tar.gz"));
        assert!(entries[3].file.is_none());
    }

    #[test]
    fn selects_the_matching_filename() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let entries = parse_checksums(&format!("{a}  one.tar.gz\n{b}  two.tar.gz\n"));
        assert_eq!(
            expected_digest(&entries, "two.tar.gz").unwrap().as_deref(),
            Some(b.as_str())
        );
        assert!(
            expected_digest(&entries, "missing.tar.gz")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn anonymous_digest_is_used_only_when_unique() {
        let a = "a".repeat(64);
        let entries = parse_checksums(&format!("{a}\n"));
        assert_eq!(
            expected_digest(&entries, "anything.tar.gz")
                .unwrap()
                .as_deref(),
            Some(a.as_str())
        );
        let entries = parse_checksums(&format!("{a}\n{}\n", "b".repeat(64)));
        assert!(expected_digest(&entries, "anything.tar.gz").is_err());
    }

    #[test]
    fn checksum_assets_are_prioritised_for_the_selected_file() {
        let release = Release {
            tag_name: "v1".to_owned(),
            name: None,
            body: None,
            draft: false,
            prerelease: false,
            published_at: None,
            assets: vec![
                Asset {
                    id: 1,
                    name: "checksums.txt".to_owned(),
                    size: 10,
                    download_count: None,
                    content_type: None,
                    browser_download_url: "https://github.com/x".to_owned(),
                },
                Asset {
                    id: 2,
                    name: "tool.tar.gz.sha256".to_owned(),
                    size: 10,
                    download_count: None,
                    content_type: None,
                    browser_download_url: "https://github.com/x".to_owned(),
                },
            ],
        };
        let selected = Asset {
            id: 3,
            name: "tool.tar.gz".to_owned(),
            size: 10,
            download_count: None,
            content_type: None,
            browser_download_url: "https://github.com/x".to_owned(),
        };
        let assets = checksum_assets(&release, &selected);
        assert_eq!(assets[0].name, "tool.tar.gz.sha256");
    }
}
