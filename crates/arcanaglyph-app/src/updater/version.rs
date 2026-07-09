// crates/arcanaglyph-app/src/updater/version.rs
//
// Разбор и сравнение версий релизов (SemVer major.minor.patch).

/// Парсер git tag в `(major, minor, patch)`. Pre-release / build suffixes
/// (`-rc1`, `+build.5`) возвращают None — мы не показываем пользователю
/// нестабильные релизы.
pub fn parse_release_tag(tag: &str) -> Option<(u32, u32, u32)> {
    let stripped = tag.strip_prefix('v').unwrap_or(tag);
    if stripped.contains('-') || stripped.contains('+') {
        return None;
    }
    let parts: Vec<&str> = stripped.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    Some((parts[0].parse().ok()?, parts[1].parse().ok()?, parts[2].parse().ok()?))
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_release_tag(latest), parse_release_tag(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clean_tag() {
        assert_eq!(parse_release_tag("v1.7.0"), Some((1, 7, 0)));
        assert_eq!(parse_release_tag("1.7.0"), Some((1, 7, 0)));
        assert_eq!(parse_release_tag("v1.6.10"), Some((1, 6, 10)));
        assert_eq!(parse_release_tag("v0.0.1"), Some((0, 0, 1)));
    }

    #[test]
    fn parse_skip_pre_release() {
        assert_eq!(parse_release_tag("v1.7.0-rc1"), None);
        assert_eq!(parse_release_tag("v1.7.0-beta.1"), None);
    }

    #[test]
    fn parse_skip_build_metadata() {
        assert_eq!(parse_release_tag("v1.7.0+build.5"), None);
    }

    #[test]
    fn parse_invalid() {
        assert_eq!(parse_release_tag(""), None);
        assert_eq!(parse_release_tag("v1.7"), None);
        assert_eq!(parse_release_tag("not-a-version"), None);
        assert_eq!(parse_release_tag("v1.a.b"), None);
    }

    #[test]
    fn newer_basic() {
        assert!(is_newer("1.7.0", "1.6.9"));
        assert!(is_newer("2.0.0", "1.99.99"));
    }

    #[test]
    fn newer_minor_double_digit() {
        // Sanity-check на наивный лексикографический compare.
        assert!(is_newer("1.6.10", "1.6.9"));
    }

    #[test]
    fn newer_same_or_older() {
        assert!(!is_newer("1.6.9", "1.6.9"));
        assert!(!is_newer("1.6.8", "1.6.9"));
    }

    #[test]
    fn newer_invalid_returns_false() {
        assert!(!is_newer("rc-foo", "1.6.9"));
        assert!(!is_newer("1.6.9", "rc-foo"));
        assert!(!is_newer("v1.7.0-rc1", "1.6.9"));
    }
}
