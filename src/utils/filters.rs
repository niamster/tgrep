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
