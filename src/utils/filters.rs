use anyhow::Error;
use log::debug;

use crate::utils::patterns::{Pattern, PatternSet};

#[derive(Clone, Default)]
pub struct Filters {
    patterns: PatternSet,
}

impl Filters {
    pub fn new(strings: &[String]) -> Result<Self, Error> {
        let mut patterns = PatternSet::new("/");
        for pattern in strings {
            let pattern = if pattern.starts_with("**/") {
                pattern.to_owned()
            } else {
                "**/".to_owned() + pattern
            };
            let transformed = Pattern::new(&pattern)?;
            debug!("Transformed filter {:?} -> {:?}", pattern, transformed);
            patterns.push(transformed, false);
        }
        Ok(Filters { patterns })
    }

    pub fn matches(&self, path: &str) -> bool {
        self.patterns.matches(path, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_prefixed_glob_filters() {
        let filters = Filters::new(&["*.rs".to_string(), "src/*.toml".to_string()]).unwrap();

        assert!(filters.matches("/tmp/project/src/main.rs"));
        assert!(filters.matches("/tmp/project/src/Cargo.toml"));
        assert!(!filters.matches("/tmp/project/src/main.py"));
        assert!(!filters.matches("/tmp/project/Cargo.toml"));
    }

    #[test]
    fn preserves_double_star_prefixes() {
        let filters = Filters::new(&["**/README.md".to_string()]).unwrap();

        assert!(filters.matches("/tmp/project/README.md"));
        assert!(filters.matches("/tmp/project/docs/README.md"));
        assert!(!filters.matches("/tmp/project/docs/README.txt"));
    }
}
