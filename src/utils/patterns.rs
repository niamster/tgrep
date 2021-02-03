use std::path::PathBuf;

use glob::Pattern;
use quicli::prelude::*;

use crate::utils::lines::ToLines;

#[derive(Clone)]
pub struct Patterns {
    whitelist: Vec<Pattern>,
    blacklist: Vec<Pattern>,
}

impl Patterns {
    pub fn new() -> Self {
        Patterns {
            whitelist: Vec::new(),
            blacklist: Vec::new(),
        }
    }

    pub fn extend(&mut self, other: &Patterns) {
        self.whitelist.extend_from_slice(&other.whitelist);
        self.whitelist.dedup();
        self.blacklist.extend_from_slice(&other.blacklist);
        self.blacklist.dedup();
    }

    pub fn is_excluded(&self, path: &str, is_dir: bool) -> bool {
        let mut paths = vec![path.to_owned()];
        if is_dir {
            // FIXME: extra ugly stuff to w/a globs and avoid entering dir
            paths.push(path.to_owned() + "/");
            paths.push(path.to_owned() + "/*");
        }
        for pattern in &self.whitelist {
            for path in &paths {
                if pattern.matches(path.as_str()) {
                    return false;
                }
            }
        }
        for pattern in &self.blacklist {
            for path in &paths {
                if pattern.matches(path.as_str()) {
                    return true;
                }
            }
        }
        false
    }
}

pub trait ToPatterns {
    fn to_patterns(&self) -> (Patterns, Patterns);
}

impl ToPatterns for Vec<String> {
    fn to_patterns(&self) -> (Patterns, Patterns) {
        let (mut root_patterns, mut patterns) = (Patterns::new(), Patterns::new());
        for pattern in self {
            let pattern = pattern.trim();
            if pattern.starts_with('#') || pattern.is_empty() {
                continue;
            }
            let is_root = pattern.starts_with('/');
            let pattern = if is_root {
                pattern.strip_prefix('/').unwrap()
            } else {
                pattern
            };
            let pattern = if pattern.ends_with('/') {
                pattern.to_owned() + "*"
            } else {
                pattern.to_string()
            };
            let whitelist = pattern.starts_with('!');
            let pattern = if whitelist {
                &pattern[1..]
            } else {
                pattern.as_str()
            };
            // FIXME: either implement better support of https://git-scm.com/docs/gitignore or use existing lib
            #[allow(clippy::collapsible_if)]
            match Pattern::new(pattern) {
                Ok(pattern) => {
                    if is_root {
                        if whitelist {
                            root_patterns.whitelist.push(pattern)
                        } else {
                            root_patterns.blacklist.push(pattern)
                        }
                    } else {
                        if whitelist {
                            patterns.whitelist.push(pattern)
                        } else {
                            patterns.blacklist.push(pattern)
                        }
                    }
                }
                Err(e) => error!("Failed to compile pattern '{}': {}", pattern, e),
            }
        }
        (root_patterns, patterns)
    }
}

impl ToPatterns for PathBuf {
    fn to_patterns(&self) -> (Patterns, Patterns) {
        match self.to_lines() {
            Ok(contents) => {
                let mut lines = Vec::new();
                for line in contents {
                    if let Ok(line) = line {
                        lines.push(line);
                    }
                }
                lines.to_patterns()
            }
            Err(e) => {
                error!("Failed to read file with pattern: {}", e);
                (Patterns::new(), Patterns::new())
            }
        }
    }
}

impl ToPatterns for Vec<PathBuf> {
    fn to_patterns(&self) -> (Patterns, Patterns) {
        let (mut root_patterns, mut patterns) = (Patterns::new(), Patterns::new());
        for path in self {
            let (root_pat, pat) = path.to_patterns();
            root_patterns.extend(&root_pat);
            patterns.extend(&pat);
        }
        (root_patterns, patterns)
    }
}
