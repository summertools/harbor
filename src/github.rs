//! GitHub REST client with strict URL handling and bounded responses.
//!
//! Only the public GitHub API is contacted by default. Release downloads are
//! validated separately by [`crate::download`], which refuses non-GitHub hosts
//! and follows redirects manually.

use std::env;
use std::io::Read;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{Error, Result};

const MAX_API_BYTES: u64 = 8 * 1024 * 1024;
const API_VERSION: &str = "2022-11-28";

/// Fully qualified GitHub repository reference.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RepoRef {
    pub owner: String,
    pub name: String,
}

impl RepoRef {
    pub fn parse(value: &str) -> Result<Self> {
        let (owner, name) = crate::config::parse_repo_ref(value)?;
        Ok(Self { owner, name })
    }

    #[must_use]
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

impl std::fmt::Display for RepoRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.full_name())
    }
}

/// A repository returned by the GitHub API.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RepoSummary {
    pub id: u64,
    pub name: String,
    pub full_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub fork: bool,
    #[serde(default)]
    pub default_branch: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
}

/// A GitHub release.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

impl Release {
    #[must_use]
    pub fn display_name(&self) -> &str {
        self.name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.tag_name)
    }

    #[must_use]
    pub fn date(&self) -> &str {
        self.published_at
            .as_deref()
            .and_then(|value| value.get(..10))
            .unwrap_or("-")
    }
}

/// One downloadable release asset.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Asset {
    pub id: u64,
    pub name: String,
    pub size: u64,
    #[serde(default)]
    pub download_count: Option<u64>,
    #[serde(default)]
    pub content_type: Option<String>,
    pub browser_download_url: String,
}

/// Small model used by `harbor doctor`.
#[derive(Clone, Debug, Deserialize)]
struct RateLimitResponse {
    resources: RateLimitResources,
}

#[derive(Clone, Debug, Deserialize)]
struct RateLimitResources {
    core: RateLimitCore,
}

#[derive(Clone, Debug, Deserialize)]
struct RateLimitCore {
    limit: u64,
    remaining: u64,
    reset: u64,
}

/// Read the token from `GH_TOKEN` or `GITHUB_TOKEN` without ever logging it.
#[must_use]
pub fn token_from_env() -> Option<String> {
    ["GH_TOKEN", "GITHUB_TOKEN"]
        .iter()
        .find_map(|name| env::var(name).ok())
        .map(|token| token.trim().to_owned())
        .filter(|token| {
            !token.is_empty() && token.len() <= 512 && !token.chars().any(char::is_control)
        })
}

/// GitHub REST client.
pub struct GitHubClient {
    http: Client,
    download_http: Client,
    api_base: Url,
    token: Option<String>,
    user_agent: String,
}

impl std::fmt::Debug for GitHubClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GitHubClient")
            .field("api_base", &self.api_base)
            .field("token_present", &self.token.is_some())
            .finish_non_exhaustive()
    }
}

impl GitHubClient {
    pub fn new() -> Result<Self> {
        Self::with_token(token_from_env())
    }

    pub fn with_token(token: Option<String>) -> Result<Self> {
        let api_base = Url::parse("https://api.github.com/")?;
        Self::with_base_url(api_base, token)
    }

    /// Testing hook. Production callers should use [`Self::new`].
    pub fn with_base_url(api_base: Url, token: Option<String>) -> Result<Self> {
        let http = build_http(Duration::from_secs(90))?;
        let download_http = build_http(Duration::from_secs(30 * 60))?;
        Ok(Self {
            http,
            download_http,
            api_base,
            token,
            user_agent: format!(
                "harbor/{} (+https://github.com/summertools/harbor)",
                env!("CARGO_PKG_VERSION")
            ),
        })
    }

    #[must_use]
    pub fn token_present(&self) -> bool {
        self.token.is_some()
    }

    pub(crate) fn download_http(&self) -> &Client {
        &self.download_http
    }

    pub(crate) fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// List public repositories under an organisation. If the name is a user
    /// rather than an organisation, fall back to the user endpoint.
    pub fn list_org_repos(&self, owner: &str, limit: usize) -> Result<Vec<RepoSummary>> {
        let limit = limit.clamp(1, 300);
        let mut repos = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut endpoint = "orgs";
        for page in 1..=3 {
            if repos.len() >= limit {
                break;
            }
            let mut url = self.url(&[endpoint, owner, "repos"])?;
            url.query_pairs_mut()
                .append_pair("per_page", "100")
                .append_pair("page", &page.to_string())
                .append_pair("sort", "updated");
            let batch: Vec<RepoSummary> = match self.get_json_url(url) {
                Ok(batch) => batch,
                Err(Error::NotFound(_)) if page == 1 && endpoint == "orgs" => {
                    endpoint = "users";
                    let mut url = self.url(&[endpoint, owner, "repos"])?;
                    url.query_pairs_mut()
                        .append_pair("per_page", "100")
                        .append_pair("page", "1")
                        .append_pair("sort", "updated");
                    self.get_json_url(url)?
                }
                Err(error) => return Err(error),
            };
            if batch.is_empty() {
                break;
            }
            let count = batch.len();
            for repo in batch {
                if seen.insert(repo.full_name.to_ascii_lowercase()) {
                    repos.push(repo);
                    if repos.len() >= limit {
                        break;
                    }
                }
            }
            if count < 100 {
                break;
            }
        }
        Ok(repos)
    }

    /// Fetch repository metadata.
    pub fn repo(&self, repo: &RepoRef) -> Result<RepoSummary> {
        self.get_json_url(self.url(&["repos", &repo.owner, &repo.name])?)
    }

    /// Fetch the latest release, optionally accepting prereleases.
    pub fn latest_release(&self, repo: &RepoRef, include_prereleases: bool) -> Result<Release> {
        if !include_prereleases {
            match self.get_json_url(self.url(&[
                "repos",
                &repo.owner,
                &repo.name,
                "releases",
                "latest",
            ])?) {
                Ok(release) => return Ok(release),
                Err(Error::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
        }
        let releases = self.list_releases(repo, 10)?;
        let release = if include_prereleases {
            releases.into_iter().find(|release| !release.draft)
        } else {
            releases
                .into_iter()
                .find(|release| !release.draft && !release.prerelease)
        };
        release.ok_or_else(|| {
            Error::NotFound(format!(
                "{} has no {}release; enable include_prereleases or choose a tag explicitly",
                repo.full_name(),
                if include_prereleases {
                    "published "
                } else {
                    "stable "
                }
            ))
        })
    }

    /// Fetch a release by tag.
    pub fn release_by_tag(&self, repo: &RepoRef, tag: &str) -> Result<Release> {
        let mut url = self.url(&["repos", &repo.owner, &repo.name, "releases", "tags"])?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| Error::msg("invalid GitHub API base URL"))?;
            segments.push(tag);
        }
        self.get_json_url(url)
    }

    /// List recent releases, newest first.
    pub fn list_releases(&self, repo: &RepoRef, limit: usize) -> Result<Vec<Release>> {
        let mut url = self.url(&["repos", &repo.owner, &repo.name, "releases"])?;
        url.query_pairs_mut()
            .append_pair("per_page", &limit.clamp(1, 100).to_string());
        self.get_json_url(url)
    }

    /// Query the authenticated/core rate limit.
    pub fn rate_limit(&self) -> Result<(u64, u64, u64)> {
        let response: RateLimitResponse = self.get_json_url(self.url(&["rate_limit"])?)?;
        Ok((
            response.resources.core.limit,
            response.resources.core.remaining,
            response.resources.core.reset,
        ))
    }

    fn url(&self, segments: &[&str]) -> Result<Url> {
        let mut url = self.api_base.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| Error::msg("invalid GitHub API base URL"))?;
            path.pop_if_empty();
            for segment in segments {
                path.push(segment);
            }
        }
        Ok(url)
    }

    fn get_json_url<T: DeserializeOwned>(&self, url: Url) -> Result<T> {
        let mut request = self
            .http
            .get(url.clone())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", API_VERSION)
            .header("User-Agent", &self.user_agent);
        if let Some(token) = &self.token
            && url.host_str() == self.api_base.host_str()
        {
            request = request.bearer_auth(token);
        }
        let response = request.send()?;
        let status = response.status();
        if !status.is_success() {
            return Err(self.api_error(response, status));
        }
        let text = read_text_limited(response, MAX_API_BYTES)?;
        serde_json::from_str(&text).map_err(Error::Json)
    }

    fn api_error(&self, response: Response, status: StatusCode) -> Error {
        let reset = rate_limit_reset(&response);
        let body = read_text_limited(response, 256 * 1024).unwrap_or_default();
        let message = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| {
                body.chars()
                    .filter(|ch| !ch.is_control())
                    .take(300)
                    .collect()
            });
        if status == StatusCode::NOT_FOUND {
            return Error::NotFound(if message.is_empty() {
                "GitHub resource not found".to_owned()
            } else {
                message
            });
        }
        if status == StatusCode::UNAUTHORIZED {
            return Error::Api("authentication failed; check GH_TOKEN/GITHUB_TOKEN".to_owned());
        }
        if status == StatusCode::TOO_MANY_REQUESTS || reset.is_some() {
            return Error::RateLimited { reset };
        }
        Error::Api(format!(
            "HTTP {}: {}",
            status.as_u16(),
            if message.is_empty() {
                status.canonical_reason().unwrap_or("request failed")
            } else {
                &message
            }
        ))
    }
}

fn build_http(timeout: Duration) -> Result<Client> {
    Client::builder()
        .user_agent(format!("harbor/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(Error::Network)
}

fn rate_limit_reset(response: &Response) -> Option<String> {
    let remaining = response
        .headers()
        .get("x-ratelimit-remaining")?
        .to_str()
        .ok()?;
    if remaining != "0" {
        return None;
    }
    let reset = response
        .headers()
        .get("x-ratelimit-reset")?
        .to_str()
        .ok()?
        .parse::<u64>()
        .ok()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let seconds = reset.saturating_sub(now);
    Some(if seconds == 0 {
        "now".to_owned()
    } else if seconds < 3600 {
        format!("in {} min", seconds.div_ceil(60))
    } else {
        format!("in {} h", seconds.div_ceil(3600))
    })
}

fn read_text_limited(response: Response, limit: u64) -> Result<String> {
    let mut reader = response.take(limit.saturating_add(1));
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Error::Api(format!(
            "GitHub response exceeded {} KiB",
            limit / 1024
        )));
    }
    String::from_utf8(bytes)
        .map_err(|_| Error::Api("GitHub returned a non-UTF-8 response".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_refs_reject_path_injection() {
        assert_eq!(
            RepoRef::parse("summertools/rtop").unwrap().full_name(),
            "summertools/rtop"
        );
        assert!(RepoRef::parse("summertools").is_err());
        assert!(RepoRef::parse("a/b/c").is_err());
        assert!(RepoRef::parse("../etc/passwd").is_err());
    }

    #[test]
    fn release_display_name_and_date_are_bounded() {
        let release = Release {
            tag_name: "v1.2.3".to_owned(),
            name: Some("  ".to_owned()),
            body: None,
            draft: false,
            prerelease: false,
            published_at: Some("2026-11-11T08:04:21Z".to_owned()),
            assets: Vec::new(),
        };
        assert_eq!(release.display_name(), "v1.2.3");
        assert_eq!(release.date(), "2026-11-11");
    }

    #[test]
    fn api_base_urls_can_be_scoped_for_tests() {
        let client =
            GitHubClient::with_base_url(Url::parse("http://127.0.0.1:1234/api/").unwrap(), None)
                .unwrap();
        let url = client.url(&["repos", "owner", "name"]).unwrap();
        assert_eq!(url.as_str(), "http://127.0.0.1:1234/api/repos/owner/name");
    }
}
