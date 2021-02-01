use std::path::PathBuf;

use glob::Pattern;
use quicli::prelude::*;

use crate::utils::lines::ToLines;

pub trait ToPatterns {
    fn to_patterns(&self) -> (Vec<Pattern>, Vec<Pattern>);
}

impl ToPatterns for Vec<String> {
    fn to_patterns(&self) -> (Vec<Pattern>, Vec<Pattern>) {
        let (mut root_patterns, mut patterns) = (Vec::new(), Vec::new());
        for pattern in self {
            let pattern = pattern.trim();
            if pattern.starts_with("#") || pattern.is_empty() {
                continue;
            }
            let is_root = pattern.starts_with("/");
            let pattern = if is_root {
                pattern.strip_prefix("/").unwrap()
            } else {
                pattern
            };
            let pattern = if pattern.ends_with("/") {
                pattern.to_owned() + "*"
            } else {
                pattern.to_string()
            };
            if pattern.starts_with("!") {
                error!("Pattern ('{}) negation is not (yet) supported", pattern);
                continue;
            }
            // FIXME: either implement better support of https://git-scm.com/docs/gitignore or use existing lib
            match Pattern::new(pattern.as_str()) {
                Ok(pattern) => {
                    if is_root {
                        root_patterns.push(pattern)
                    } else {
                        patterns.push(pattern)
                    }
                }
                Err(e) => error!("Failed to compile pattern '{}': {}", pattern, e),
            }
        }
        (root_patterns, patterns)
    }
}

impl ToPatterns for PathBuf {
    fn to_patterns(&self) -> (Vec<Pattern>, Vec<Pattern>) {
        match self.to_lines() {
            Ok(lines) => {
                let mut patterns = Vec::new();
                for line in lines {
                    if let Ok(line) = line {
                        patterns.push(line);
                    }
                }
                patterns.to_patterns()
            }
            Err(e) => {
                error!("Failed to read file with pattern: {}", e);
                (vec![], vec![])
            }
        }
    }
}

impl ToPatterns for Vec<PathBuf> {
    fn to_patterns(&self) -> (Vec<Pattern>, Vec<Pattern>) {
        let (mut root_patterns, mut patterns) = (Vec::new(), Vec::new());
        for path in self {
            let (root_pat, pat) = path.to_patterns();
            root_patterns.extend(root_pat);
            patterns.extend(pat);
        }
        (root_patterns, patterns)
    }
}
