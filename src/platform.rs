//! Platform detection, asset tokens, and executable-directory resolution.

use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};

/// Operating-system family used for asset matching.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Os {
    Linux,
    Macos,
    Windows,
    Other,
}

impl Os {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Windows => "windows",
            Self::Other => "unknown",
        }
    }
}

/// CPU architecture used for asset matching.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Arch {
    X86_64,
    Aarch64,
    X86,
    Armv7,
    Riscv64,
    Other,
}

impl Arch {
    pub const fn label(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
            Self::X86 => "x86",
            Self::Armv7 => "armv7",
            Self::Riscv64 => "riscv64",
            Self::Other => "unknown",
        }
    }

    const fn patterns(self) -> &'static [&'static str] {
        match self {
            Self::X86_64 => &["x86_64", "x86-64", "amd64", "x64"],
            Self::Aarch64 => &["aarch64", "arm64"],
            Self::X86 => &["i686", "i386"],
            Self::Armv7 => &["armv7", "armhf"],
            Self::Riscv64 => &["riscv64"],
            Self::Other => &[],
        }
    }
}

/// C library / runtime ABI used for asset matching.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Libc {
    Gnu,
    Musl,
    Msvc,
    Unknown,
}

impl Libc {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Gnu => "gnu",
            Self::Musl => "musl",
            Self::Msvc => "msvc",
            Self::Unknown => "unknown",
        }
    }
}

/// Current compile-time/runtime platform.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Platform {
    pub os: Os,
    pub arch: Arch,
    pub libc: Libc,
}

impl Platform {
    /// Detect the running platform. `cfg!` is deliberate: asset matching must
    /// describe the binary we can actually execute, not merely the host kernel.
    #[must_use]
    pub fn current() -> Self {
        let os = if cfg!(target_os = "linux") {
            Os::Linux
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Other
        };
        let arch = if cfg!(target_arch = "x86_64") {
            Arch::X86_64
        } else if cfg!(target_arch = "aarch64") {
            Arch::Aarch64
        } else if cfg!(target_arch = "x86") {
            Arch::X86
        } else if cfg!(target_arch = "arm") {
            Arch::Armv7
        } else if cfg!(target_arch = "riscv64") {
            Arch::Riscv64
        } else {
            Arch::Other
        };
        let libc = if os == Os::Windows {
            if cfg!(target_env = "msvc") {
                Libc::Msvc
            } else {
                Libc::Gnu
            }
        } else if cfg!(target_env = "musl") {
            Libc::Musl
        } else if cfg!(target_env = "gnu") {
            Libc::Gnu
        } else {
            Libc::Unknown
        };
        Self { os, arch, libc }
    }

    #[must_use]
    pub fn label(self) -> String {
        match self.os {
            Os::Linux => format!("linux-{}-{}", self.arch.label(), self.libc.label()),
            _ => format!("{}-{}", self.os.label(), self.arch.label()),
        }
    }

    /// True when a lower-case asset name contains another OS token.
    #[must_use]
    pub fn has_foreign_os(self, lower: &str) -> bool {
        let tokens = split_tokens(lower);
        let linux = tokens.contains(&"linux");
        let mac = tokens
            .iter()
            .any(|t| matches!(*t, "darwin" | "macos" | "osx"));
        let windows = tokens
            .iter()
            .any(|t| matches!(*t, "windows" | "win32" | "win64" | "win"));
        match self.os {
            Os::Linux => mac || windows,
            Os::Macos => linux || windows,
            Os::Windows => linux || mac,
            Os::Other => false,
        }
    }

    /// True when a lower-case asset name contains another architecture token.
    #[must_use]
    pub fn has_foreign_arch(self, lower: &str) -> bool {
        let flags = arch_flags(lower);
        match self.arch {
            Arch::X86_64 => flags.aarch64 || flags.x86 || flags.armv7 || flags.riscv64,
            Arch::Aarch64 => flags.x86_64 || flags.x86 || flags.armv7 || flags.riscv64,
            Arch::X86 => flags.x86_64 || flags.aarch64 || flags.armv7 || flags.riscv64,
            Arch::Armv7 => flags.x86_64 || flags.aarch64 || flags.x86 || flags.riscv64,
            Arch::Riscv64 => flags.x86_64 || flags.aarch64 || flags.x86 || flags.armv7,
            Arch::Other => false,
        }
    }

    /// True when the asset explicitly names the current OS.
    #[must_use]
    pub fn names_os(self, lower: &str) -> bool {
        let tokens = split_tokens(lower);
        match self.os {
            Os::Linux => tokens.contains(&"linux"),
            Os::Macos => tokens
                .iter()
                .any(|t| matches!(*t, "darwin" | "macos" | "osx" | "apple")),
            Os::Windows => tokens
                .iter()
                .any(|t| matches!(*t, "windows" | "win32" | "win64" | "win")),
            Os::Other => false,
        }
    }

    /// True when the asset explicitly names the current architecture.
    #[must_use]
    pub fn names_arch(self, lower: &str) -> bool {
        let flags = arch_flags(lower);
        match self.arch {
            Arch::X86_64 => flags.x86_64,
            Arch::Aarch64 => flags.aarch64,
            Arch::X86 => flags.x86,
            Arch::Armv7 => flags.armv7,
            Arch::Riscv64 => flags.riscv64,
            Arch::Other => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ArchFlags {
    x86_64: bool,
    aarch64: bool,
    x86: bool,
    armv7: bool,
    riscv64: bool,
}

fn arch_flags(lower: &str) -> ArchFlags {
    let tokens = split_tokens(lower);
    let has = |patterns: &[&str]| patterns.iter().any(|pattern| lower.contains(pattern));
    let x86_64 = has(Arch::X86_64.patterns());
    let aarch64 = has(Arch::Aarch64.patterns());
    let armv7 = has(Arch::Armv7.patterns()) || tokens.contains(&"arm");
    let riscv64 = has(Arch::Riscv64.patterns());
    let x86 = !x86_64
        && (has(Arch::X86.patterns()) || (tokens.contains(&"x86") && !lower.contains("x86_64")));
    ArchFlags {
        x86_64,
        aarch64,
        x86,
        armv7,
        riscv64,
    }
}

/// Split an asset name into lower-case alphanumeric tokens.
#[must_use]
pub fn split_tokens(lower: &str) -> Vec<&str> {
    lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect()
}

/// A resolved directory that can receive installed executables.
#[derive(Clone, Debug)]
pub struct BinDir {
    pub path: PathBuf,
    pub in_path: bool,
}

impl BinDir {
    #[must_use]
    pub fn display(&self) -> String {
        compact_path(&self.path)
    }
}

/// Resolve the executable directory in this order: CLI, config, environment,
/// then a writable user directory that is already on PATH.
pub fn resolve_bin_dir(explicit: Option<&Path>, configured: Option<&Path>) -> Result<BinDir> {
    if let Some(path) = explicit {
        return prepare_bin_dir(&expand_tilde(path), true);
    }
    if let Some(path) = configured {
        return prepare_bin_dir(&expand_tilde(path), true);
    }
    if let Some(value) = env::var_os("HARBOR_BIN_DIR").filter(|value| !value.is_empty()) {
        return prepare_bin_dir(&expand_tilde(Path::new(&value)), true);
    }

    let home = home_dir().ok_or_else(|| Error::msg("could not determine the home directory"))?;
    let mut candidates = vec![
        home.join(".local").join("bin"),
        home.join("bin"),
        home.join(".cargo").join("bin"),
        data_dir().join("harbor").join("bin"),
    ];
    if let Some(local) = env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty()) {
        candidates.insert(0, PathBuf::from(local).join("harbor").join("bin"));
    }
    for candidate in &candidates {
        if is_writable_directory(candidate) && path_contains(candidate) {
            return prepare_bin_dir(candidate, false);
        }
    }
    // No known user directory is on PATH. Prefer the conventional one and
    // create it; the caller will print a PATH hint rather than editing shell
    // startup files behind the user's back.
    prepare_bin_dir(&candidates[0], false)
}

/// Create and validate a bin directory immediately before installing.
pub fn ensure_bin_dir(path: &Path) -> Result<PathBuf> {
    Ok(prepare_bin_dir(path, true)?.path)
}

fn prepare_bin_dir(path: &Path, explicit: bool) -> Result<BinDir> {
    let path = absolute_path(path)?;
    if path == Path::new("/") {
        return Err(Error::msg("refusing to install directly into /"));
    }
    if let Ok(metadata) = fs::symlink_metadata(&path)
        && !metadata.is_dir()
        && !metadata.file_type().is_symlink()
    {
        return Err(Error::msg(format!("{} is not a directory", path.display())));
    }
    fs::create_dir_all(&path).map_err(|error| {
        Error::msg(format!(
            "cannot create install directory {}: {error}",
            path.display()
        ))
    })?;
    let canonical = fs::canonicalize(&path)?;
    if canonical.parent().is_none() {
        return Err(Error::msg(
            "refusing to install directly into a filesystem root",
        ));
    }
    if home_dir().is_some_and(|home| fs::canonicalize(home).is_ok_and(|home| home == canonical)) {
        return Err(Error::msg(
            "refusing to install directly into the home directory; choose a bin subdirectory",
        ));
    }
    let metadata = fs::metadata(&canonical)?;
    if !metadata.is_dir() {
        return Err(Error::msg(format!(
            "{} is not a directory",
            canonical.display()
        )));
    }
    if !is_writable_directory(&canonical) {
        let hint = if explicit {
            "choose a directory you own or run the install command without elevated privileges"
        } else {
            "set bin_dir in the config or use --bin-dir with a writable directory"
        };
        return Err(Error::msg(format!(
            "install directory {} is not writable; {hint}",
            canonical.display()
        )));
    }
    let in_path = path_contains(&canonical);
    Ok(BinDir {
        path: canonical,
        in_path,
    })
}

/// Resolve the install directory without creating it. Used by `--dry-run` so
/// planning never mutates the filesystem.
pub fn planned_bin_dir(explicit: Option<&Path>, configured: Option<&Path>) -> Result<BinDir> {
    if let Some(path) = explicit {
        return planned_path(&expand_tilde(path));
    }
    if let Some(path) = configured {
        return planned_path(&expand_tilde(path));
    }
    if let Some(value) = env::var_os("HARBOR_BIN_DIR").filter(|value| !value.is_empty()) {
        return planned_path(&expand_tilde(Path::new(&value)));
    }
    let home = home_dir().ok_or_else(|| Error::msg("could not determine the home directory"))?;
    let mut candidates = vec![
        home.join(".local").join("bin"),
        home.join("bin"),
        home.join(".cargo").join("bin"),
        data_dir().join("harbor").join("bin"),
    ];
    if let Some(local) = env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty()) {
        candidates.insert(0, PathBuf::from(local).join("harbor").join("bin"));
    }
    for candidate in &candidates {
        if is_writable_directory(candidate) && path_contains(candidate) {
            return planned_path(candidate);
        }
    }
    planned_path(&candidates[0])
}

fn planned_path(path: &Path) -> Result<BinDir> {
    let path = absolute_path(path)?;
    if let Ok(canonical) = fs::canonicalize(&path) {
        if canonical.parent().is_none() {
            return Err(Error::msg(
                "refusing to install directly into a filesystem root",
            ));
        }
        if home_dir().is_some_and(|home| fs::canonicalize(home).is_ok_and(|home| home == canonical))
        {
            return Err(Error::msg(
                "refusing to install directly into the home directory; choose a bin subdirectory",
            ));
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            if !canonical.is_dir() {
                return Err(Error::msg(format!("{} is not a directory", path.display())));
            }
        } else if !metadata.is_dir() {
            return Err(Error::msg(format!("{} is not a directory", path.display())));
        }
        let in_path = path_contains(&canonical);
        return Ok(BinDir {
            path: canonical,
            in_path,
        });
    }
    if path.parent().is_none() {
        return Err(Error::msg(
            "refusing to install directly into a filesystem root",
        ));
    }
    let in_path = path_contains(&path);
    Ok(BinDir { path, in_path })
}

/// Create a unique-looking but deterministic temp path in `dir`.
pub fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(normalize(path))
    } else {
        let cwd = env::current_dir()?;
        Ok(normalize(&cwd.join(path)))
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Test writability without leaving a file behind.
#[must_use]
pub fn is_writable_directory(path: &Path) -> bool {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };
    if !metadata.is_dir() {
        return false;
    }
    tempfile::Builder::new()
        .prefix(".harbor-write-test-")
        .tempfile_in(path)
        .is_ok()
}

/// True when `path` is one of the entries in PATH (after normalisation).
#[must_use]
pub fn path_contains(path: &Path) -> bool {
    let Ok(target) = fs::canonicalize(path) else {
        return env::split_paths(&env::var_os("PATH").unwrap_or_default())
            .any(|entry| normalize(&entry) == normalize(path));
    };
    env::split_paths(&env::var_os("PATH").unwrap_or_default()).any(|entry| {
        fs::canonicalize(&entry).map_or_else(
            |_| normalize(&entry) == normalize(path),
            |entry| entry == target,
        )
    })
}

/// Expand a leading `~` using the current home directory.
#[must_use]
pub fn expand_tilde(path: &Path) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path.to_path_buf();
    };
    if text == "~" {
        return home_dir().unwrap_or_else(|| path.to_path_buf());
    }
    if let Some(rest) = text.strip_prefix("~/")
        && let Some(home) = home_dir()
    {
        return home.join(rest);
    }
    if let Some(rest) = text.strip_prefix("~\\")
        && let Some(home) = home_dir()
    {
        return home.join(rest);
    }
    path.to_path_buf()
}

/// Replace the home prefix with `~` for concise UI paths.
#[must_use]
pub fn compact_path(path: &Path) -> String {
    let text = path.display().to_string();
    if let Some(home) = home_dir() {
        let home = home.display().to_string();
        if let Some(rest) = text.strip_prefix(&home)
            && (rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\'))
        {
            return format!("~{rest}");
        }
    }
    text
}

/// Validate a name that will be created inside an executable directory.
pub fn validate_binary_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::InvalidBinaryName(name.to_owned()));
    }
    if name.len() > 128 {
        return Err(Error::InvalidBinaryName(name.to_owned()));
    }
    if name == "." || name == ".." {
        return Err(Error::InvalidBinaryName(name.to_owned()));
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err(Error::InvalidBinaryName(name.to_owned()));
    }
    if name.chars().any(char::is_control) {
        return Err(Error::InvalidBinaryName(name.to_owned()));
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '+'))
    {
        return Err(Error::InvalidBinaryName(name.to_owned()));
    }
    if cfg!(windows) {
        let upper = name.to_ascii_uppercase();
        let stem = upper.split('.').next().unwrap_or("");
        let reserved = [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ];
        if reserved.contains(&stem) || name.ends_with('.') || name.ends_with(' ') {
            return Err(Error::InvalidBinaryName(name.to_owned()));
        }
    }
    Ok(name.to_owned())
}

/// Ensure a Windows target keeps its `.exe` suffix.
#[must_use]
pub fn executable_name(name: &str) -> String {
    if cfg!(windows) && !name.to_ascii_lowercase().ends_with(".exe") {
        format!("{name}.exe")
    } else {
        name.to_owned()
    }
}

#[must_use]
pub fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| env::var_os("USERPROFILE").filter(|value| !value.is_empty()))
        .map(PathBuf::from)
}

#[must_use]
pub fn config_dir() -> PathBuf {
    if let Some(value) = env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(value).join("harbor");
    }
    if cfg!(windows)
        && let Some(value) = env::var_os("APPDATA").filter(|value| !value.is_empty())
    {
        return PathBuf::from(value).join("harbor");
    }
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("harbor")
}

#[must_use]
pub fn state_dir() -> PathBuf {
    if let Some(value) = env::var_os("XDG_STATE_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(value).join("harbor");
    }
    if cfg!(windows)
        && let Some(value) = env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty())
    {
        return PathBuf::from(value).join("harbor").join("state");
    }
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local")
        .join("state")
        .join("harbor")
}

#[must_use]
pub fn cache_dir() -> PathBuf {
    if let Some(value) = env::var_os("XDG_CACHE_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(value).join("harbor");
    }
    if cfg!(windows)
        && let Some(value) = env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty())
    {
        return PathBuf::from(value).join("harbor").join("cache");
    }
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache")
        .join("harbor")
}

#[must_use]
pub fn data_dir() -> PathBuf {
    if let Some(value) = env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(value).join("harbor");
    }
    if cfg!(windows)
        && let Some(value) = env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty())
    {
        return PathBuf::from(value).join("harbor");
    }
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local")
        .join("share")
        .join("harbor")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenisation_ignores_punctuation() {
        assert_eq!(
            split_tokens("rtop-v0.1.3-x86_64-unknown-linux-musl.tar.gz"),
            vec![
                "rtop", "v0", "1", "3", "x86", "64", "unknown", "linux", "musl", "tar", "gz"
            ]
        );
    }

    #[test]
    fn platform_scores_do_not_accept_foreign_architectures() {
        let platform = Platform {
            os: Os::Linux,
            arch: Arch::X86_64,
            libc: Libc::Gnu,
        };
        let asset = "tool-v1-x86_64-unknown-linux-musl.tar.gz";
        assert!(platform.names_os(asset));
        assert!(!platform.has_foreign_arch(asset));
        assert!(platform.has_foreign_arch("tool-aarch64-apple-darwin.tar.gz"));
        assert!(platform.has_foreign_os("tool-aarch64-apple-darwin.tar.gz"));
    }

    #[test]
    fn explicit_bin_dirs_are_created_but_root_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("bin");
        let resolved = resolve_bin_dir(Some(&target), None).unwrap();
        assert!(resolved.path.is_absolute());
        assert!(resolved.path.exists());
        assert!(resolve_bin_dir(Some(Path::new("/")), None).is_err());
    }

    #[test]
    fn planned_bin_dir_does_not_touch_the_filesystem() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("planned-bin");
        let planned = planned_bin_dir(Some(&target), None).unwrap();
        assert_eq!(planned.path, target);
        assert!(!target.exists());
    }

    #[test]
    fn binary_names_are_restricted_to_safe_components() {
        assert!(validate_binary_name("rtop").is_ok());
        assert!(validate_binary_name("rtop.exe").is_ok());
        assert!(validate_binary_name("../rtop").is_err());
        assert!(validate_binary_name("a/b").is_err());
        assert!(validate_binary_name("a\u{1b}b").is_err());
        assert!(validate_binary_name(" ").is_err());
    }

    #[test]
    fn compact_path_understands_home() {
        if let Some(home) = home_dir() {
            assert!(compact_path(&home.join("bin").join("rtop")).starts_with('~'));
        }
    }
}
