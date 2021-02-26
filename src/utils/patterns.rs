use std::{path::PathBuf, rc::Rc};

use anyhow::Error;
use log::{debug, error, trace};

use crate::utils::lines::LinesReader;

// From https://git-scm.com/docs/gitignore
//
// 0.1 A blank line matches no files, so it can serve as a separator for readability.
// 0.2 Trailing spaces are ignored unless they are escaped with a backslash.
//
// 1.1. A line starting with # serves as a comment.
// 1.2. Put a backslash in front of the first hash for patterns that begin with a hash.
//
// 2.1. The slash / is used as the directory separator.
// 2.2. Separators may occur at the beginning, middle or end of the pattern.
// 2.3. If there is a separator at the beginning or middle (or both) of the pattern,
//      then the pattern is relative to the directory level of the particular .gitignore file itself.
//      Otherwise the pattern may also match at any level below the .gitignore level.
// 2.4. If there is a separator at the end of the pattern then the pattern will only match directories,
//      otherwise the pattern can match both files and directories.
// For example, a pattern doc/frotz/ matches doc/frotz directory, but not a/doc/frotz directory.
// However frotz/ matches frotz and a/frotz that is a directory.
//
// 3. An optional prefix "!" negates the pattern.
//    Aany matching file excluded by a previous pattern will become included again.
//
// 4.1 An asterisk "*" matches anything except a slash.
// 4.2 The character "?" matches any one character except "/".
// 4.3 The range notation, e.g. [a-zA-Z], can be used to match one of the characters in a range.
//
// 5. Two consecutive asterisks ("**") in patterns matched against full pathname may have special meaning.
// 5.1 A leading "**" followed by a slash means match in all directories.
//     For example, "**/foo" matches file or directory "foo" anywhere,
//     the same as pattern "foo". "**/foo/bar" matches file or directory "bar" anywhere
//     that is directly under directory "foo".
// 5.2 A trailing "/**" matches everything inside.
//     For example, "abc/**" matches all files inside directory "abc",
//     relative to the location of the .gitignore file, with infinite depth.
// 5.3 A slash followed by two consecutive asterisks then a slash matches zero or more directories.
//     For example, "a/**/b" matches "a/b", "a/x/b", "a/x/y/b" and so on.
// 5.4 Other consecutive asterisks are considered regular asterisks and
//     will match according to the previous rules.

#[derive(Clone, PartialEq)]
struct Pattern {
    pattern: Rc<glob::Pattern>,
    root: Rc<String>,
    dir_only: bool,
}

impl Pattern {
    fn new(pattern: &str, root: &str, dir_only: bool) -> Result<Self, Error> {
        Ok(Pattern {
            pattern: Rc::new(glob::Pattern::new(&pattern)?),
            root: Rc::new(root.trim_end_matches('/').to_owned()),
            dir_only,
        })
    }

    fn matches(&self, path: &str, is_dir: bool) -> bool {
        if self.dir_only && !is_dir {
            return false;
        }
        let truncated = path.trim_start_matches(&*self.root);
        let matches = self.pattern.matches(truncated);
        trace!(
            "Testing {:?} against {:?}: {} (root:{:?} truncated:{:?})",
            path,
            self.pattern.as_str(),
            if matches { "match" } else { "mismatch" },
            self.root,
            truncated,
        );
        matches
    }
}

#[derive(Clone, Default)]
pub struct Patterns {
    whitelist: Vec<Pattern>,
    blacklist: Vec<Pattern>,
}

impl Patterns {
    pub fn new(root: &str, strings: &[String]) -> Self {
        let mut patterns: Patterns = Default::default();
        for pattern in strings {
            let orig = pattern.clone();
            let pattern = pattern.trim_start();
            let pattern = if pattern.ends_with("\\ ") {
                pattern
            } else {
                let pattern = pattern.trim_end();
                if pattern == "\\" {
                    " "
                } else {
                    pattern
                }
            };
            if pattern.starts_with('#') || pattern.is_empty() {
                continue;
            }
            let pattern = pattern.replace("\\ ", " ");
            let whitelist = pattern.starts_with('!');
            let pattern = if whitelist {
                &pattern[1..]
            } else {
                pattern.as_str()
            };
            let pattern = if pattern.starts_with("\\#") || pattern.starts_with("\\!") {
                pattern.strip_prefix('\\').unwrap()
            } else {
                pattern
            };
            // `./.git` == `/.git`
            let pattern = if pattern.starts_with("./") {
                pattern.strip_prefix('.').unwrap()
            } else {
                pattern
            };
            let root_only = pattern.starts_with('/')
                || (pattern.contains('/')
                    && !pattern.ends_with('/')
                    && !pattern.starts_with("**/")
                    && !pattern.contains("/**/"));
            let dir_only = pattern.ends_with('/') || pattern.ends_with("/*");
            let pattern = pattern.trim_end_matches('/');
            let pattern = pattern.trim_end_matches("/*");
            let pattern = if root_only {
                "/".to_owned() + pattern.trim_start_matches('/')
            } else if !pattern.starts_with("**/") {
                "**/".to_owned() + pattern
            } else {
                pattern.to_owned()
            };
            debug!(
                "{:?} -> {:?} (root:{:?}, dir:{}, whitelist:{})",
                orig, pattern, root, dir_only, whitelist,
            );
            #[allow(clippy::collapsible_if)]
            match Pattern::new(&pattern, root, dir_only) {
                Ok(pattern) => {
                    if whitelist {
                        patterns.whitelist.push(pattern)
                    } else {
                        patterns.blacklist.push(pattern)
                    }
                }
                Err(e) => error!("Failed to compile pattern '{}': {}", pattern, e),
            }
        }
        patterns
    }

    pub fn extend(&mut self, other: &Patterns) {
        self.whitelist.extend_from_slice(&other.whitelist);
        self.whitelist.dedup();
        self.blacklist.extend_from_slice(&other.blacklist);
        self.blacklist.dedup();
    }

    pub fn is_excluded(&self, path: &str, is_dir: bool) -> Option<String> {
        for pattern in &self.whitelist {
            if pattern.matches(path, is_dir) {
                return None;
            }
        }
        for pattern in &self.blacklist {
            if pattern.matches(path, is_dir) {
                return Some(pattern.pattern.to_string());
            }
        }
        None
    }
}

pub trait ToPatterns {
    fn to_patterns(&self) -> Patterns;
}

impl ToPatterns for PathBuf {
    fn to_patterns(&self) -> Patterns {
        match self.lines() {
            Ok(mut contents) => {
                let mut lines = Vec::new();
                while let Some(line) = contents.next() {
                    lines.push(line.to_owned());
                }
                let root = self.as_path().parent().unwrap();
                let root = root.canonicalize().unwrap();
                let root = root.to_str().unwrap();
                Patterns::new(root, &lines)
            }
            Err(e) => {
                error!("Failed to read file with pattern: {}", e);
                Default::default()
            }
        }
    }
}

impl ToPatterns for Vec<PathBuf> {
    fn to_patterns(&self) -> Patterns {
        let mut patterns: Patterns = Default::default();
        for path in self {
            patterns.extend(&path.to_patterns());
        }
        patterns
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use env_logger;

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn gitignore() {
        init();

        let strings = vec![
            // 0.
            "   ",
            "\\  ",
            " bim  ",
            " bam\\ \\ ",
            // 1.
            "#boom",
            r"\#kaboom",
            // 2.
            "/foo",
            "./zoo",
            "bar/baz",
            "/baz/buz/",
            "/baz/buzz/*",
            "baz/qux/",
            // 3.
            "toto*",
            "!totoro",
            r"\!totoro",
            // 5.
            "**/tata",
            "titi/**/titi",
            "tutu/**",
        ]
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<String>>();
        for root in vec!["/", "/r/"] {
            let patterns = Patterns::new(root, &strings);
            let mkpath = |path| root.to_owned() + path;

            for tf in vec![true, false] {
                // 0.
                assert_eq!(true, patterns.is_excluded(&mkpath(" "), tf).is_some());
                assert_eq!(true, patterns.is_excluded(&mkpath("bim"), tf).is_some());
                assert_eq!(false, patterns.is_excluded(&mkpath("bim  "), tf).is_some());
                assert_eq!(false, patterns.is_excluded(&mkpath("bam"), tf).is_some());
                assert_eq!(true, patterns.is_excluded(&mkpath("bam  "), tf).is_some());

                // 1.
                assert_eq!(false, patterns.is_excluded(&mkpath("#boom"), tf).is_some());
                assert_eq!(true, patterns.is_excluded(&mkpath("#kaboom"), tf).is_some());

                // 2.
                assert_eq!(true, patterns.is_excluded(&mkpath("foo"), tf).is_some());
                assert_eq!(
                    false,
                    patterns.is_excluded(&mkpath("moo/foo"), tf).is_some()
                );

                assert_eq!(true, patterns.is_excluded(&mkpath("zoo"), tf).is_some());

                assert_eq!(true, patterns.is_excluded(&mkpath("bar/baz"), tf).is_some());
                assert_eq!(
                    false,
                    patterns.is_excluded(&mkpath("buz/bar/baz"), tf).is_some()
                );

                assert_eq!(tf, patterns.is_excluded(&mkpath("baz/buz"), tf).is_some());
                assert_eq!(tf, patterns.is_excluded(&mkpath("baz/buzz"), tf).is_some());

                assert_eq!(tf, patterns.is_excluded(&mkpath("baz/qux"), tf).is_some());
                assert_eq!(tf, patterns.is_excluded(&mkpath("baz/qux"), tf).is_some());

                // 3.
                assert_eq!(
                    true,
                    patterns.is_excluded(&mkpath("totorino"), tf).is_some()
                );
                assert_eq!(false, patterns.is_excluded(&mkpath("totoro"), tf).is_some());
                assert_eq!(true, patterns.is_excluded(&mkpath("!totoro"), tf).is_some());

                // 5.
                assert_eq!(
                    true,
                    patterns
                        .is_excluded(&mkpath("boo/baz/boz/tata"), tf)
                        .is_some()
                );

                assert_eq!(
                    true,
                    patterns
                        .is_excluded(&mkpath("titi/baz/boz/titi"), tf)
                        .is_some()
                );

                assert_eq!(
                    false,
                    patterns
                        .is_excluded(&mkpath("titi/tutu/baz/boz"), tf)
                        .is_some()
                );
                assert_eq!(
                    true,
                    patterns
                        .is_excluded(&mkpath("tutu/baz/boz/titi"), tf)
                        .is_some()
                );
            }
        }
    }
}
