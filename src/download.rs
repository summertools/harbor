//! Bounded, host-restricted release downloads with progress reporting.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use reqwest::blocking::Client;
use reqwest::header::LOCATION;
use sha2::{Digest, Sha256};
use url::Url;

use crate::error::{Error, Result};

/// A downloaded temporary file plus its SHA-256 digest.
#[derive(Debug)]
pub struct Download {
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
}

/// Download `url` into `dir`, following only GitHub-hosted HTTPS redirects.
///
/// `max_bytes` is enforced before and during streaming. The returned file is
/// deleted automatically when its [`PathBuf`] is removed by the caller.
pub fn download(
    http: &Client,
    token: Option<&str>,
    url: &str,
    dir: &Path,
    max_bytes: u64,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<Download> {
    let mut last_error = None;
    for attempt in 0..3 {
        match download_once(http, token, url, dir, max_bytes, &mut progress) {
            Ok(download) => return Ok(download),
            Err(error @ Error::Network(_)) if attempt < 2 => {
                last_error = Some(error);
                std::thread::sleep(std::time::Duration::from_millis(250 * (attempt + 1) as u64));
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or_else(|| Error::msg("download failed")))
}

fn download_once(
    http: &Client,
    token: Option<&str>,
    url: &str,
    dir: &Path,
    max_bytes: u64,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<Download> {
    let max_bytes = max_bytes.max(1);
    let mut current = Url::parse(url)?;
    for _ in 0..=5 {
        validate_download_url(&current)?;
        let mut request = http
            .get(current.clone())
            .header("Accept", "application/octet-stream")
            .header(
                "User-Agent",
                format!(
                    "harbor/{} (+https://github.com/summertools/harbor)",
                    env!("CARGO_PKG_VERSION")
                ),
            );
        if let Some(token) = token
            && token_host(&current)
        {
            request = request.bearer_auth(token);
        }
        let mut response = request
            .send()
            .map_err(|error| Error::Network(error.without_url()))?;
        let status = response.status();
        if status.is_redirection() {
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| Error::msg("GitHub redirect is missing a Location header"))?;
            current = current.join(location)?;
            continue;
        }
        if !status.is_success() {
            let mut body = String::new();
            let _ = response.take(128 * 1024).read_to_string(&mut body);
            let message: String = body
                .chars()
                .filter(|ch| !ch.is_control())
                .take(300)
                .collect();
            return Err(Error::Api(format!(
                "download failed with HTTP {}: {}",
                status.as_u16(),
                if message.is_empty() {
                    status.canonical_reason().unwrap_or("request failed")
                } else {
                    &message
                }
            )));
        }
        if response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().starts_with("text/html"))
        {
            return Err(Error::Api(
                "download returned an HTML page instead of a release asset".to_owned(),
            ));
        }
        let content_length = response.content_length();
        if let Some(length) = content_length
            && length > max_bytes
        {
            return Err(Error::DownloadTooLarge {
                limit_mb: max_bytes.div_ceil(1024 * 1024),
            });
        }

        let mut temp = tempfile::Builder::new()
            .prefix("harbor-download-")
            .tempfile_in(dir)?;
        let mut hasher = Sha256::new();
        let mut total = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        progress(0, content_length);
        loop {
            let read = response.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            total = total.saturating_add(read as u64);
            if total > max_bytes {
                return Err(Error::DownloadTooLarge {
                    limit_mb: max_bytes.div_ceil(1024 * 1024),
                });
            }
            hasher.update(&buffer[..read]);
            temp.write_all(&buffer[..read])?;
            progress(total, content_length);
        }
        if let Some(expected) = content_length
            && expected != total
        {
            return Err(Error::Api(format!(
                "download ended at {total} bytes but GitHub announced {expected}"
            )));
        }
        temp.as_file_mut().sync_all()?;
        let (_file, path) = temp.keep().map_err(|error| Error::Io(error.error))?;
        return Ok(Download {
            path,
            sha256: hex::encode(hasher.finalize()),
            bytes: total,
        });
    }
    Err(Error::Api("too many GitHub download redirects".to_owned()))
}

/// Verify that a URL is a safe GitHub release download target.
pub fn validate_download_url(url: &Url) -> Result<()> {
    if url.username() != "" || url.password().is_some() {
        return Err(Error::UntrustedHost(url.to_string()));
    }
    let host = url
        .host_str()
        .ok_or_else(|| Error::UntrustedHost(url.to_string()))?
        .to_ascii_lowercase();
    if url.scheme() != "https" || !allowed_host(&host) {
        return Err(Error::UntrustedHost(host));
    }
    Ok(())
}

#[must_use]
pub fn allowed_host(host: &str) -> bool {
    matches!(
        host,
        "github.com"
            | "api.github.com"
            | "objects.githubusercontent.com"
            | "release-assets.githubusercontent.com"
            | "github-releases.githubusercontent.com"
            | "codeload.github.com"
            | "github-production-release-asset-2e65be.s3.amazonaws.com"
    ) || host.ends_with(".githubusercontent.com")
        || (host.starts_with("github-production-release-asset-")
            && host.ends_with(".s3.amazonaws.com"))
}

fn token_host(url: &Url) -> bool {
    url.host_str().is_some_and(|host| {
        let host = host.to_ascii_lowercase();
        matches!(host.as_str(), "api.github.com" | "github.com")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_github_hosts_are_allowed() {
        for valid in [
            "https://github.com/owner/repo/releases/download/v1/tool.tar.gz",
            "https://api.github.com/repos/owner/repo/releases/assets/1",
            "https://release-assets.githubusercontent.com/signed/path",
            "https://objects.githubusercontent.com/signed/path",
            "https://github-production-release-asset-2e65be.s3.amazonaws.com/key",
        ] {
            assert!(
                validate_download_url(&Url::parse(valid).unwrap()).is_ok(),
                "{valid}"
            );
        }
        for invalid in [
            "http://github.com/owner/repo",
            "https://example.com/tool.tar.gz",
            "https://127.0.0.1/tool",
            "https://github.com.evil.example/tool",
            "https://user:pass@github.com/tool",
        ] {
            assert!(
                validate_download_url(&Url::parse(invalid).unwrap()).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn plain_http_and_loopback_are_rejected() {
        assert!(validate_download_url(&Url::parse("http://127.0.0.1:8080/file").unwrap()).is_err());
        assert!(validate_download_url(&Url::parse("http://localhost:8080/file").unwrap()).is_err());
    }
}
