//! glob-style patterns: `pdf-*`, `search_*`, `*`

pub fn pattern_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return value.starts_with(prefix);
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        return value.ends_with(suffix);
    }
    pattern == value
}

pub fn any_pattern_match(patterns: &[String], value: &str) -> bool {
    patterns.iter().any(|p| pattern_match(p, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_matches_all() {
        assert!(pattern_match("*", "anything"));
    }

    #[test]
    fn prefix_suffix_globs() {
        assert!(pattern_match("pdf-*", "pdf-tools"));
        assert!(!pattern_match("pdf-*", "doc-pdf"));
        assert!(pattern_match("*-report", "weekly-report"));
        assert!(pattern_match("search_*", "search_issues"));
    }
}
