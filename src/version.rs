//! Version comparison used only for automatic updates.
//!
//! Manual updates still follow the release selected by GitHub. Auto-update is
//! more conservative: if a tag cannot be parsed as SemVer, Harbor refuses to
//! guess whether it is newer and leaves the decision to the user.

/// Return `Some(true)` when `latest` is a newer SemVer than `installed`,
/// `Some(false)` when they are equal or older, and `None` when either tag is
/// not parseable.
#[must_use]
pub fn is_newer(installed: &str, latest: &str) -> Option<bool> {
    let installed = normalize(installed)?;
    let latest = normalize(latest)?;
    Some(latest.cmp_precedence(&installed) == std::cmp::Ordering::Greater)
}

fn normalize(tag: &str) -> Option<semver::Version> {
    let tag = tag.trim();
    let tag = tag
        .strip_prefix('v')
        .or_else(|| tag.strip_prefix('V'))
        .unwrap_or(tag);
    semver::Version::parse(tag).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_common_release_tags() {
        assert_eq!(is_newer("v1.2.3", "v1.2.4"), Some(true));
        assert_eq!(is_newer("1.2.3", "v1.2.3"), Some(false));
        assert_eq!(is_newer("v2.0.0", "v1.9.9"), Some(false));
        assert_eq!(is_newer("v1.2.3", "v1.10.0"), Some(true));
        assert_eq!(is_newer("v1.0.0", "v1.0.1-rc.1"), Some(true));
        // Build metadata is not part of SemVer precedence.
        assert_eq!(is_newer("v1.0.0+old", "v1.0.0+new"), Some(false));
        assert_eq!(is_newer("v1.0.0", "v1.0.0+build"), Some(false));
    }

    #[test]
    fn refuses_to_guess_for_non_semver_tags() {
        assert_eq!(is_newer("release-1", "release-2"), None);
        assert_eq!(is_newer("v1.2", "v1.3"), None);
    }
}
