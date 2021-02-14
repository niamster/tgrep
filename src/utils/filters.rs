use anyhow::Error;

#[derive(Clone, Default)]
pub struct Filters {
    patterns: Vec<glob::Pattern>,
}

impl Filters {
    pub fn new(strings: &[String]) -> Result<Self, Error> {
        let mut filters = Filters { patterns: vec![] };
        for pattern in strings {
            let pattern = if pattern.starts_with("**/") {
                pattern.to_owned()
            } else {
                "**/".to_owned() + pattern
            };
            filters.patterns.push(glob::Pattern::new(&pattern)?);
        }
        Ok(filters)
    }

    pub fn matches(&self, path: &str) -> bool {
        for pattern in &self.patterns {
            if pattern.matches(path) {
                return true;
            }
        }
        false
    }
}
