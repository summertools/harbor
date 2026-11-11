//! Safe archive extraction and executable discovery.
//!
//! Extraction never trusts archive paths. Absolute paths, `..`, symlinks,
//! hard links, devices, and FIFOs are rejected; files are created with
//! `create_new` inside a fresh private directory, and both the file count and
//! total extracted bytes are capped.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};
use crate::release::AssetKind;

/// Resource limits for one archive.
#[derive(Clone, Copy, Debug)]
pub struct ExtractLimits {
    pub max_files: usize,
    pub max_bytes: u64,
}

impl Default for ExtractLimits {
    fn default() -> Self {
        Self {
            max_files: 10_000,
            max_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}

/// One executable-looking file found in an archive.
#[derive(Clone, Debug)]
pub struct BinaryCandidate {
    pub path: PathBuf,
    pub relative: PathBuf,
    pub name: String,
    pub score: i32,
}

/// Verify the asset looks like the container implied by its name.
pub fn verify_kind(path: &Path, kind: AssetKind) -> Result<()> {
    let mut file = File::open(path)?;
    let mut magic = [0u8; 64];
    let read = file.read(&mut magic)?;
    let magic = &magic[..read];
    if kind == AssetKind::Raw {
        if magic.is_empty() {
            return Err(Error::Archive("raw release asset is empty".to_owned()));
        }
        if magic.starts_with(&[0x1f, 0x8b])
            || magic.starts_with(b"PK\x03\x04")
            || magic.starts_with(b"PK\x05\x06")
        {
            return Err(Error::Archive(
                "raw release asset is an archive; use a supported archive asset".to_owned(),
            ));
        }
        let texty = magic
            .iter()
            .all(|byte| byte.is_ascii_graphic() || byte.is_ascii_whitespace());
        if texty && !magic.starts_with(b"#!") {
            return Err(Error::Archive(
                "raw release asset looks like a text file, not an executable".to_owned(),
            ));
        }
        return Ok(());
    }
    let valid = match kind {
        AssetKind::TarGz => magic.starts_with(&[0x1f, 0x8b]),
        AssetKind::Zip => {
            magic.starts_with(b"PK\x03\x04")
                || magic.starts_with(b"PK\x05\x06")
                || magic.starts_with(b"PK\x07\x08")
        }
        AssetKind::Tar => true,
        AssetKind::Raw => true,
    };
    if !valid {
        return Err(Error::Archive(format!(
            "{} does not look like a {} archive",
            path.file_name().unwrap_or_default().to_string_lossy(),
            kind.label()
        )));
    }
    Ok(())
}

/// Extract `asset` into the empty `destination` directory.
pub fn extract(
    asset: &Path,
    kind: AssetKind,
    destination: &Path,
    limits: ExtractLimits,
) -> Result<()> {
    verify_kind(asset, kind)?;
    match kind {
        AssetKind::TarGz => extract_tar_gz(asset, destination, limits),
        AssetKind::Tar => extract_tar(File::open(asset)?, destination, limits),
        AssetKind::Zip => extract_zip(asset, destination, limits),
        AssetKind::Raw => Err(Error::Archive(
            "internal error: raw binary passed to archive extraction".to_owned(),
        )),
    }
}

fn extract_tar_gz(asset: &Path, destination: &Path, limits: ExtractLimits) -> Result<()> {
    let file = File::open(asset)?;
    let decoder = flate2::read::GzDecoder::new(file);
    extract_tar(decoder, destination, limits)
}

fn extract_tar<R: Read>(reader: R, destination: &Path, limits: ExtractLimits) -> Result<()> {
    let mut archive = tar::Archive::new(reader);
    let mut files = 0usize;
    let mut bytes = 0u64;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let entry_type = entry.header().entry_type();
        let relative = safe_relative_path(&entry.path()?)?;
        if entry_type.is_dir() {
            fs::create_dir_all(destination.join(&relative))?;
            continue;
        }
        if !entry_type.is_file() {
            return Err(Error::UnsafeArchive(format!(
                "{} is not a regular file or directory",
                relative.display()
            )));
        }
        files += 1;
        if files > limits.max_files {
            return Err(Error::Archive(format!(
                "archive contains more than {} files",
                limits.max_files
            )));
        }
        let size = entry.size();
        bytes = bytes.saturating_add(size);
        if bytes > limits.max_bytes {
            return Err(Error::Archive(format!(
                "archive expands beyond {} MiB",
                limits.max_bytes / (1024 * 1024)
            )));
        }
        let path = destination.join(&relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let mode = entry.header().mode().unwrap_or(0o600);
        let copied = io::copy(&mut entry, &mut file)?;
        if copied != size {
            return Err(Error::Archive(format!(
                "{} is truncated (expected {size} bytes, got {copied})",
                relative.display()
            )));
        }
        file.flush()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let executable = mode & 0o111 != 0;
            fs::set_permissions(
                &path,
                fs::Permissions::from_mode(if executable { 0o700 } else { 0o600 }),
            )?;
        }
    }
    Ok(())
}

fn extract_zip(asset: &Path, destination: &Path, limits: ExtractLimits) -> Result<()> {
    let file = File::open(asset)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| Error::Archive(format!("cannot open zip: {error}")))?;
    let mut files = 0usize;
    let mut bytes = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| Error::Archive(format!("cannot read zip entry: {error}")))?;
        let Some(enclosed) = entry.enclosed_name() else {
            let name = entry
                .name()
                .map_or_else(|_| "<invalid>".to_owned(), |name| name.to_string());
            return Err(Error::UnsafeArchive(format!(
                "zip entry {name} escapes the archive root"
            )));
        };
        let relative = safe_relative_path(&enclosed)?;
        if entry.is_dir() {
            fs::create_dir_all(destination.join(&relative))?;
            continue;
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(Error::UnsafeArchive(format!(
                "{} is a symbolic link",
                relative.display()
            )));
        }
        files += 1;
        if files > limits.max_files {
            return Err(Error::Archive(format!(
                "archive contains more than {} files",
                limits.max_files
            )));
        }
        let size = entry.size();
        bytes = bytes.saturating_add(size);
        if bytes > limits.max_bytes {
            return Err(Error::Archive(format!(
                "archive expands beyond {} MiB",
                limits.max_bytes / (1024 * 1024)
            )));
        }
        let path = destination.join(&relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let copied = io::copy(&mut entry, &mut file)?;
        if copied != size {
            return Err(Error::Archive(format!(
                "{} is truncated (expected {size} bytes, got {copied})",
                relative.display()
            )));
        }
        file.flush()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let executable = entry.unix_mode().is_some_and(|mode| mode & 0o111 != 0);
            fs::set_permissions(
                &path,
                fs::Permissions::from_mode(if executable { 0o700 } else { 0o600 }),
            )?;
        }
    }
    Ok(())
}

/// Reject absolute paths, parent components, and empty paths.
pub fn safe_relative_path(path: &Path) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(Error::UnsafeArchive(format!(
                    "{} contains '..'",
                    path.display()
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(Error::UnsafeArchive(format!(
                    "{} is absolute",
                    path.display()
                )));
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err(Error::UnsafeArchive(
            "archive entry has an empty path".to_owned(),
        ));
    }
    Ok(out)
}

/// Find executable candidates in an extracted directory.
pub fn discover_binaries(
    root: &Path,
    repo_name: &str,
    limits: ExtractLimits,
) -> Result<Vec<BinaryCandidate>> {
    let mut candidates = Vec::new();
    let mut count = 0usize;
    walk(
        root,
        root,
        0,
        repo_name,
        limits,
        &mut count,
        &mut candidates,
    )?;
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.relative.cmp(&right.relative))
    });
    Ok(candidates)
}

fn walk(
    root: &Path,
    directory: &Path,
    depth: usize,
    repo_name: &str,
    limits: ExtractLimits,
    count: &mut usize,
    out: &mut Vec<BinaryCandidate>,
) -> Result<()> {
    if depth > 6 {
        return Ok(());
    }
    let entries = fs::read_dir(directory)?;
    for entry in entries {
        let entry = entry?;
        *count += 1;
        if *count > limits.max_files.saturating_mul(4) {
            return Err(Error::Archive("archive directory is too large".to_owned()));
        }
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if matches!(
                name.as_str(),
                "doc"
                    | "docs"
                    | "share"
                    | "man"
                    | "completions"
                    | "licenses"
                    | "license"
                    | "examples"
                    | "tests"
                    | ".github"
                    | ".git"
            ) {
                continue;
            }
            walk(root, &path, depth + 1, repo_name, limits, count, out)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let Some(file_name) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
        else {
            continue;
        };
        if is_text_or_library(&file_name) {
            continue;
        }
        let metadata = entry.metadata()?;
        if metadata.len() > 512 * 1024 * 1024 {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map(|path| path.to_path_buf())
            .unwrap_or_else(|_| PathBuf::from(&file_name));
        let lower_name = file_name.to_ascii_lowercase();
        let repo_lower = repo_name.to_ascii_lowercase();
        let stem = Path::new(&file_name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(&file_name)
            .to_ascii_lowercase();
        let executable = is_executable(&metadata);
        let name_match =
            lower_name == repo_lower || stem == repo_lower || lower_name.starts_with(&repo_lower);
        if !executable && !name_match {
            continue;
        }
        let mut score = 0i32;
        if lower_name == repo_lower || lower_name == format!("{repo_lower}.exe") {
            score += 100;
        } else if stem == repo_lower {
            score += 80;
        } else if lower_name.starts_with(&repo_lower) {
            score += 40;
        }
        if executable {
            score += 30;
        }
        if cfg!(windows) && lower_name.ends_with(".exe") {
            score += 20;
        }
        score += 20i32.saturating_sub(i32::try_from(depth).unwrap_or(20).saturating_mul(3));
        if relative.components().any(|component| {
            component
                .as_os_str()
                .to_str()
                .is_some_and(|part| part.eq_ignore_ascii_case("bin"))
        }) {
            score += 8;
        }
        out.push(BinaryCandidate {
            path,
            relative,
            name: file_name,
            score,
        });
    }
    Ok(())
}

fn is_text_or_library(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    const SUFFIXES: &[&str] = &[
        ".md", ".txt", ".json", ".toml", ".yaml", ".yml", ".html", ".css", ".js", ".sh", ".ps1",
        ".bat", ".cmd", ".1", ".gz", ".zip", ".tar", ".sha256", ".sig", ".asc", ".so", ".dylib",
        ".dll", ".rlib", ".a", ".lib", ".pdb", ".h", ".hpp", ".pc",
    ];
    if SUFFIXES.iter().any(|suffix| lower.ends_with(suffix)) {
        return true;
    }
    lower.starts_with("readme")
        || lower.starts_with("license")
        || lower.starts_with("changelog")
        || lower.starts_with("notice")
        || lower.starts_with("completion")
}

fn is_executable(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}

/// Pick one binary, optionally constrained by `--binary`.
pub fn pick_binary(
    candidates: &[BinaryCandidate],
    requested: Option<&str>,
) -> Result<BinaryCandidate> {
    if let Some(requested) = requested {
        let requested = requested.trim();
        if requested.is_empty() {
            return Err(Error::NoBinary("empty --binary value".to_owned()));
        }
        let requested_path = safe_relative_path(Path::new(requested)).ok();
        let found: Vec<&BinaryCandidate> = candidates
            .iter()
            .filter(|candidate| {
                candidate.name == requested
                    || requested_path.as_ref().is_some_and(|path| {
                        candidate.relative == *path || candidate.relative.ends_with(path)
                    })
            })
            .collect();
        return match found.first() {
            Some(candidate) => Ok((*candidate).clone()),
            None => Err(Error::NoBinary(format!(
                "no file named {requested:?} was found; available: {}",
                candidate_names(candidates)
            ))),
        };
    }
    let mut sorted: Vec<&BinaryCandidate> = candidates.iter().collect();
    sorted.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.relative.cmp(&right.relative))
    });
    let Some(first) = sorted.first().copied() else {
        return Err(Error::NoBinary("archive contains no executable".to_owned()));
    };
    if let Some(second) = sorted.get(1)
        && second.score == first.score
        && second.name != first.name
    {
        return Err(Error::AmbiguousBinary(candidate_names(candidates)));
    }
    Ok(first.clone())
}

fn candidate_names(candidates: &[BinaryCandidate]) -> String {
    candidates
        .iter()
        .take(8)
        .map(|candidate| candidate.relative.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        assert!(safe_relative_path(Path::new("../evil")).is_err());
        assert!(safe_relative_path(Path::new("/etc/passwd")).is_err());
        assert!(safe_relative_path(Path::new("a/../b")).is_err());
        assert_eq!(
            safe_relative_path(Path::new("dir/tool")).unwrap(),
            PathBuf::from("dir/tool")
        );
    }

    fn make_tar_gz(path: &Path, entries: &[(&str, &[u8], u32)]) {
        let file = File::create(path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, data, mode) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(*mode);
            header.set_cksum();
            builder.append_data(&mut header, name, *data).unwrap();
        }
        let encoder = builder.into_inner().unwrap();
        encoder.finish().unwrap();
    }

    #[test]
    fn raw_text_assets_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let html = temp.path().join("not-a-binary");
        fs::write(&html, b"<!doctype html><html>error</html>").unwrap();
        assert!(verify_kind(&html, AssetKind::Raw).is_err());
        let script = temp.path().join("script");
        fs::write(&script, b"#!/bin/sh\necho hi\n").unwrap();
        assert!(verify_kind(&script, AssetKind::Raw).is_ok());
    }

    #[test]
    fn extracts_a_safe_tar_and_finds_the_binary() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("tool.tar.gz");
        make_tar_gz(
            &archive,
            &[
                ("tool-v1/bin/mytool", b"#!/bin/sh\n", 0o755),
                ("tool-v1/README.md", b"readme", 0o644),
            ],
        );
        let out = temp.path().join("out");
        fs::create_dir(&out).unwrap();
        extract(&archive, AssetKind::TarGz, &out, ExtractLimits::default()).unwrap();
        let candidates = discover_binaries(&out, "mytool", ExtractLimits::default()).unwrap();
        assert_eq!(candidates[0].name, "mytool");
        assert!(
            candidates[0]
                .relative
                .ends_with(Path::new("bin").join("mytool"))
        );
    }

    #[test]
    fn rejects_tar_links() {
        let temp = tempfile::tempdir().unwrap();
        let out = temp.path().join("out");
        fs::create_dir(&out).unwrap();

        let link_archive = temp.path().join("link.tar.gz");
        let file = File::create(&link_archive).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_link_name("../../etc/passwd").unwrap();
        header.set_cksum();
        builder.append_data(&mut header, "link", &[][..]).unwrap();
        let encoder = builder.into_inner().unwrap();
        encoder.finish().unwrap();
        assert!(matches!(
            extract(
                &link_archive,
                AssetKind::TarGz,
                &out,
                ExtractLimits::default()
            ),
            Err(Error::UnsafeArchive(_))
        ));
    }

    #[test]
    fn rejects_zip_path_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("evil.zip");
        let file = File::create(&archive).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer.start_file("../evil", options).unwrap();
        writer.write_all(b"owned").unwrap();
        writer.finish().unwrap();
        let out = temp.path().join("out");
        fs::create_dir(&out).unwrap();
        assert!(matches!(
            extract(&archive, AssetKind::Zip, &out, ExtractLimits::default()),
            Err(Error::UnsafeArchive(_))
        ));
        assert!(!temp.path().join("evil").exists());
    }

    #[test]
    fn picks_the_repo_named_binary() {
        let candidates = vec![
            BinaryCandidate {
                path: PathBuf::from("/tmp/README"),
                relative: PathBuf::from("README"),
                name: "README".to_owned(),
                score: 0,
            },
            BinaryCandidate {
                path: PathBuf::from("/tmp/tool"),
                relative: PathBuf::from("tool"),
                name: "tool".to_owned(),
                score: 100,
            },
        ];
        assert_eq!(pick_binary(&candidates, None).unwrap().name, "tool");
        assert_eq!(
            pick_binary(&candidates, Some("README")).unwrap().name,
            "README"
        );
    }
}
