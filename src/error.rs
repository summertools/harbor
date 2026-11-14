//! Error type shared by the library and CLI.

use std::path::PathBuf;

use thiserror::Error;

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong while discovering, downloading, verifying, or
/// installing a release.
#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),

    #[error("invalid configuration: {0}")]
    Config(String),

    #[error("invalid source: {0}")]
    Source(String),

    #[error("GitHub API error: {0}")]
    Api(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("no release: {0}")]
    NoRelease(String),

    #[error(
        "GitHub API rate limit exceeded{detail}",
        detail = .reset.as_deref().map(|reset| format!("; resets {reset}")).unwrap_or_default()
    )]
    RateLimited { reset: Option<String> },

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("invalid URL: {0}")]
    Url(#[from] url::ParseError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("could not find a compatible release asset for {platform}")]
    NoAsset { platform: String },

    #[error("multiple compatible release assets; choose one with --asset: {0}")]
    AmbiguousAsset(String),

    #[error(
        "unsupported release asset {0:?}: expected a .tar.gz, .tgz, .tar, .zip, .exe, or raw binary"
    )]
    UnsupportedAsset(String),

    #[error("download exceeds the configured limit of {limit_mb} MiB")]
    DownloadTooLarge { limit_mb: u64 },

    #[error("download is not from an allowed GitHub host: {0}")]
    UntrustedHost(String),

    #[error("SHA-256 mismatch for {asset}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        asset: String,
        expected: String,
        actual: String,
    },

    #[error("no SHA-256 checksum is published for {0}")]
    ChecksumMissing(String),

    #[error("checksum file {0} does not contain a usable SHA-256 digest")]
    ChecksumInvalid(String),

    #[error("archive error: {0}")]
    Archive(String),

    #[error("unsafe archive entry: {0}")]
    UnsafeArchive(String),

    #[error("archive {0} contains no installable executable")]
    NoBinary(String),

    #[error("multiple executables in the archive; choose one with --binary: {0}")]
    AmbiguousBinary(String),

    #[error("invalid binary name {0:?}")]
    InvalidBinaryName(String),

    #[error("refusing to overwrite {path}: it is not managed by harbor (use --force)")]
    UnmanagedTarget { path: PathBuf },

    #[error("refusing to replace {0}: it is not a regular file")]
    UnsafeTarget(PathBuf),

    #[error("install directory {0} is not writable")]
    UnwritableBinDir(PathBuf),

    #[error("another harbor install is already using {0}")]
    Locked(PathBuf),

    #[error("operation cancelled")]
    Cancelled,
}

impl Error {
    /// Convenience constructor for message-only failures.
    pub fn msg(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}
