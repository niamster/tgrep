use std::{default::Default, path::PathBuf, sync::Arc};

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
    pattern: Arc<glob::Pattern>,
}

impl Pattern {
    fn new(pattern: &str) -> Result<Self, Error> {
        Ok(Pattern {
            pattern: Arc::new(glob::Pattern::new(&pattern)?),
        })
    }

    fn matches(&self, path: &str) -> bool {
        let matches = self.pattern.matches(path);
        trace!(
            "Testing {:?} against {:?}: {}",
            path,
            self.pattern.as_str(),
            if matches { "match" } else { "mismatch" },
        );
        matches
    }
}

#[derive(Clone, PartialEq, Default)]
struct PatternSet {
    root: Arc<String>,
    dir_only: Vec<Pattern>,
    all: Vec<Pattern>,
}

impl PatternSet {
    fn new(root: &str) -> Self {
        PatternSet {
            root: Arc::new(root.trim_end_matches('/').to_owned()),
            ..Default::default()
        }
    }

    fn push(&mut self, pattern: Pattern, dir_only: bool) {
        if dir_only {
            self.dir_only.push(pattern);
        } else {
            self.all.push(pattern);
        }
    }

    fn matches(&self, path: &str, is_dir: bool) -> bool {
        // NOTE: this is faster than `path.trim_start_matches(&*self.root)`
        let truncated = if path.len() >= self.root.len() && path[..self.root.len()] == *self.root {
            &path[self.root.len()..]
        } else {
            path
        };
        if is_dir {
            let matches = self
                .dir_only
                .iter()
                .any(|pattern| pattern.matches(truncated));
            if matches {
                return true;
            }
        }
        self.all.iter().any(|pattern| pattern.matches(truncated))
    }
}

#[derive(Clone, Default)]
pub struct Patterns {
    whitelist: Vec<PatternSet>,
    blacklist: Vec<PatternSet>,
}

impl Patterns {
    fn parse(root: &str, pattern: &str) -> Option<(anyhow::Result<Pattern>, bool, bool)> {
        let orig = pattern;
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
            return None;
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
        Some((Pattern::new(&pattern), whitelist, dir_only))
    }

    pub fn new(root: &str, strings: &[String]) -> Self {
        let mut whitelist = PatternSet::new(root);
        let mut blacklist = PatternSet::new(root);
        for pattern in strings {
            match Self::parse(root, pattern) {
                Some((Ok(pattern), is_whitelisted, dir_only)) => {
                    if is_whitelisted {
                        whitelist.push(pattern, dir_only)
                    } else {
                        blacklist.push(pattern, dir_only)
                    }
                }
                Some((Err(e), _, _)) => error!("Failed to compile pattern '{}': {}", pattern, e),
                None => {}
            }
        }
        let mut patterns: Patterns = Default::default();
        patterns.whitelist.push(whitelist);
        patterns.blacklist.push(blacklist);
        patterns
    }

    pub fn extend(&mut self, other: &Patterns) {
        self.whitelist.extend_from_slice(&other.whitelist);
        self.whitelist.dedup();
        self.blacklist.extend_from_slice(&other.blacklist);
        self.blacklist.dedup();
    }

    pub fn is_excluded(&self, path: &str, is_dir: bool) -> bool {
        if self
            .whitelist
            .iter()
            .any(|pattern| pattern.matches(path, is_dir))
        {
            return false;
        }
        self.blacklist
            .iter()
            .any(|pattern| pattern.matches(path, is_dir))
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
                assert_eq!(true, patterns.is_excluded(&mkpath(" "), tf));
                assert_eq!(true, patterns.is_excluded(&mkpath("bim"), tf));
                assert_eq!(false, patterns.is_excluded(&mkpath("bim  "), tf));
                assert_eq!(false, patterns.is_excluded(&mkpath("bam"), tf));
                assert_eq!(true, patterns.is_excluded(&mkpath("bam  "), tf));

                // 1.
                assert_eq!(false, patterns.is_excluded(&mkpath("#boom"), tf));
                assert_eq!(true, patterns.is_excluded(&mkpath("#kaboom"), tf));

                // 2.
                assert_eq!(true, patterns.is_excluded(&mkpath("foo"), tf));
                assert_eq!(false, patterns.is_excluded(&mkpath("moo/foo"), tf));

                assert_eq!(true, patterns.is_excluded(&mkpath("zoo"), tf));

                assert_eq!(true, patterns.is_excluded(&mkpath("bar/baz"), tf));
                assert_eq!(false, patterns.is_excluded(&mkpath("buz/bar/baz"), tf));

                assert_eq!(tf, patterns.is_excluded(&mkpath("baz/buz"), tf));
                assert_eq!(tf, patterns.is_excluded(&mkpath("baz/buzz"), tf));

                assert_eq!(tf, patterns.is_excluded(&mkpath("baz/qux"), tf));
                assert_eq!(tf, patterns.is_excluded(&mkpath("baz/qux"), tf));

                // 3.
                assert_eq!(true, patterns.is_excluded(&mkpath("totorino"), tf));
                assert_eq!(false, patterns.is_excluded(&mkpath("totoro"), tf));
                assert_eq!(true, patterns.is_excluded(&mkpath("!totoro"), tf));

                // 5.
                assert_eq!(true, patterns.is_excluded(&mkpath("boo/baz/boz/tata"), tf));

                assert_eq!(true, patterns.is_excluded(&mkpath("titi/baz/boz/titi"), tf));

                assert_eq!(
                    false,
                    patterns.is_excluded(&mkpath("titi/tutu/baz/boz"), tf)
                );
                assert_eq!(true, patterns.is_excluded(&mkpath("tutu/baz/boz/titi"), tf));
            }
        }
    }
}
