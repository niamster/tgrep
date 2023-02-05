use std::{default::Default, fmt, path::PathBuf, sync::Arc};

use anyhow::Error;
use log::{debug, error, trace};
use regex::Regex;

use crate::utils::lines::LinesReader;

extern "C" {
    fn memmem(
        haystack: *const u8,
        hlen: libc::size_t,
        needle: *const u8,
        nlen: libc::size_t,
    ) -> *const u8;
}

fn find_in_string(haystack: &str, needle: &str) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    let res = unsafe {
        memmem(
            haystack.as_ptr(),
            haystack.len(),
            needle.as_ptr(),
            needle.len(),
        )
    };
    if res.is_null() {
        return None;
    }
    let dist = unsafe { res.offset_from(haystack.as_ptr()) as usize };
    if dist >= haystack.len() {
        return None;
    }
    Some(dist)
}

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

#[derive(PartialEq)]
enum PatternType {
    Any,
    Exact(String),
    Prefix(String),
    Suffix(String),
    StarSuffix(String),
    PrefixStar(String),
    DStarTextDStarText((String, String)),
    Glob(glob::Pattern),
    // Potentially more cases:
    // 1. "**/foo/**"
}

impl fmt::Debug for PatternType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use PatternType::*;
        match self {
            Any => formatter.write_str("Any"),
            Exact(pattern) => formatter.write_fmt(format_args!("Exact({:?})", pattern)),
            Prefix(pattern) => formatter.write_fmt(format_args!("Prefix({:?})", pattern)),
            Suffix(pattern) => formatter.write_fmt(format_args!("Suffix({:?})", pattern)),
            StarSuffix(pattern) => formatter.write_fmt(format_args!("StarSuffix({:?})", pattern)),
            PrefixStar(pattern) => formatter.write_fmt(format_args!("PrefixStar({:?})", pattern)),
            DStarTextDStarText((first, second)) => formatter.write_fmt(format_args!(
                "DStarTextDStarText({:?}, {:?})",
                first, second
            )),
            Glob(pattern) => formatter.write_fmt(format_args!("Glob({:?})", pattern.as_str())),
        }
    }
}

#[derive(Clone, PartialEq)]
pub(crate) struct Pattern {
    pattern: Arc<PatternType>,
}

impl Pattern {
    pub(crate) fn new(pattern: &str) -> Result<Self, Error> {
        let transformed = if pattern == "*" || pattern == "**/*" {
            PatternType::Any
        } else if let Some(capture) = Self::re(r"**/\*([:]*)", pattern) {
            // `**/*foo`
            PatternType::StarSuffix(capture)
        } else if let Some(capture) = Self::re(r"**(/[:]*)", pattern) {
            // `**/foo`
            PatternType::Suffix(capture)
        } else if let Some(capture) = Self::re(r"**/([:]*)\*", pattern) {
            // `**/foo*`
            PatternType::PrefixStar(capture)
        } else if let Some(capture) = Self::re(r"(/[:]*)\*", pattern) {
            // `/foo*`
            PatternType::Prefix(capture)
        } else if let Some((first, second)) = Self::re2(r"**/([:]*/)**(/[:]*)", pattern) {
            // `**/foo/**/bar`
            PatternType::DStarTextDStarText((first, second))
        } else if let Some(capture) = Self::re(r"(/[:]*)", pattern) {
            // `/foo`
            PatternType::Exact(capture)
        } else {
            PatternType::Glob(glob::Pattern::new(pattern)?)
        };
        Ok(Pattern {
            pattern: Arc::new(transformed),
        })
    }

    fn re_prepare(regex: &str) -> String {
        let regex = regex.replace("**", r"\*\*");
        let regex = regex.replace("[:]", r"[^\]\[*?]");
        format!("^{}$", regex)
    }

    fn re(regex: &str, pattern: &str) -> Option<String> {
        let regex = Self::re_prepare(regex);
        Regex::new(&regex)
            .unwrap()
            .captures(pattern)
            .map(|capture| capture.get(1).unwrap().as_str().to_string())
    }

    fn re2(regex: &str, pattern: &str) -> Option<(String, String)> {
        let regex = Self::re_prepare(regex);
        Regex::new(&regex)
            .unwrap()
            .captures(pattern)
            .map(|capture| {
                (
                    capture.get(1).unwrap().as_str().to_string(),
                    capture.get(2).unwrap().as_str().to_string(),
                )
            })
    }

    fn matches(&self, path: &str) -> bool {
        let matches = match &*self.pattern {
            PatternType::Any => true,
            PatternType::Exact(pattern) => pattern == path,
            PatternType::Prefix(pattern) => {
                path.len() > pattern.len() && &path[..pattern.len()] == pattern
            }
            PatternType::Suffix(pattern) => {
                path.len() >= pattern.len()
                    && path.is_char_boundary(path.len() - pattern.len())
                    && &path[path.len() - pattern.len()..] == pattern
            }
            PatternType::PrefixStar(pattern) => {
                if let Some(pos) = memchr::memrchr(b'/', path.as_bytes()) {
                    let path = &path[pos + 1..];
                    path.len() > pattern.len() && &path[..pattern.len()] == pattern
                } else {
                    false
                }
            }
            PatternType::StarSuffix(pattern) => {
                path.len() > pattern.len()
                    && path.as_bytes()[path.len() - pattern.len() - 1] != b'/'
                    && &path[path.len() - pattern.len()..] == pattern
            }
            PatternType::DStarTextDStarText((first, second)) => {
                if path.len() > first.len() + second.len() {
                    if let Some(pos) = find_in_string(path, first) {
                        let path = &path[pos + first.len()..];
                        path.len() > second.len() && &path[path.len() - second.len()..] == second
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            PatternType::Glob(pattern) => pattern.matches(path),
        };
        trace!(
            "Testing {:?} against {:?}: {}",
            path,
            self.pattern,
            if matches { "match" } else { "mismatch" },
        );
        matches
    }
}

impl fmt::Debug for Pattern {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_fmt(format_args!("{:?}", self.pattern))
    }
}

#[derive(Clone, PartialEq, Default)]
pub(crate) struct PatternSet {
    root: Arc<String>,
    dir_only: Vec<Pattern>,
    all: Vec<Pattern>,
}

impl PatternSet {
    pub(crate) fn new(root: &str) -> Self {
        PatternSet {
            root: Arc::new(root.trim_end_matches('/').to_owned()),
            ..Default::default()
        }
    }

    pub(crate) fn push(&mut self, pattern: Pattern, dir_only: bool) {
        if dir_only {
            self.dir_only.push(pattern);
        } else {
            self.all.push(pattern);
        }
    }

    pub(crate) fn matches(&self, path: &str, is_dir: bool) -> bool {
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
        let transformed = Pattern::new(&pattern);
        debug!(
            "Transformed pattern {:?} -> {:?} -> {:?} (root:{:?}, dir:{}, whitelist:{})",
            orig, pattern, transformed, root, dir_only, whitelist,
        );
        Some((transformed, whitelist, dir_only))
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
        patterns.whitelist.dedup();
        patterns.blacklist.push(blacklist);
        patterns.blacklist.dedup();
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
    fn to_patterns(&self) -> anyhow::Result<Patterns>;
}

impl ToPatterns for PathBuf {
    fn to_patterns(&self) -> anyhow::Result<Patterns> {
        let mut contents = self.lines()?;
        let mut lines = Vec::new();
        while let Some(line) = contents.next() {
            lines.push(line.to_owned());
        }
        let root = self.as_path().parent().unwrap();
        let root = root.canonicalize().unwrap();
        let root = root.to_str().unwrap();
        Ok(Patterns::new(root, &lines))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use env_logger;

    fn init() {
        let _ = env_logger::builder()
            .is_test(match std::env::var("RUST_LOG_CAPTURE") {
                Ok(val) if val == "n" => false,
                _ => true,
            })
            .try_init();
    }

    #[test]
    fn test_find_in_string() {
        let test = |haystack: &str, needle: &str| {
            assert_eq!(haystack.find(needle), find_in_string(haystack, needle));
        };
        test("foozoo", "bar");
        test("foozoo", "zoo");
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
            "/zoomzoom*",
            "toto*",
            "*.ro",
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

            for is_dir in vec![true, false] {
                // 0.
                assert_eq!(true, patterns.is_excluded(&mkpath(" "), is_dir));
                assert_eq!(true, patterns.is_excluded(&mkpath("bim"), is_dir));
                assert_eq!(false, patterns.is_excluded(&mkpath("bim  "), is_dir));
                assert_eq!(false, patterns.is_excluded(&mkpath("bam"), is_dir));
                assert_eq!(true, patterns.is_excluded(&mkpath("bam  "), is_dir));

                // 1.
                assert_eq!(false, patterns.is_excluded(&mkpath("#boom"), is_dir));
                assert_eq!(true, patterns.is_excluded(&mkpath("#kaboom"), is_dir));

                // 2.
                assert_eq!(true, patterns.is_excluded(&mkpath("foo"), is_dir));
                assert_eq!(false, patterns.is_excluded(&mkpath("moo/foo"), is_dir));

                assert_eq!(true, patterns.is_excluded(&mkpath("zoo"), is_dir));

                assert_eq!(true, patterns.is_excluded(&mkpath("bar/baz"), is_dir));
                assert_eq!(false, patterns.is_excluded(&mkpath("buz/bar/baz"), is_dir));

                assert_eq!(is_dir, patterns.is_excluded(&mkpath("baz/buz"), is_dir));
                assert_eq!(is_dir, patterns.is_excluded(&mkpath("baz/buzz"), is_dir));

                assert_eq!(is_dir, patterns.is_excluded(&mkpath("baz/qux"), is_dir));
                assert_eq!(is_dir, patterns.is_excluded(&mkpath("baz/qux"), is_dir));

                // 3.
                assert_eq!(true, patterns.is_excluded(&mkpath("zoomzoomzoom"), is_dir));
                assert_eq!(false, patterns.is_excluded(&mkpath("zoomzoom"), is_dir));
                assert_eq!(true, patterns.is_excluded(&mkpath("totorino"), is_dir));
                assert_eq!(false, patterns.is_excluded(&mkpath("toto"), is_dir));
                assert_eq!(false, patterns.is_excluded(&mkpath("totoro"), is_dir));
                assert_eq!(true, patterns.is_excluded(&mkpath("!totoro"), is_dir));
                assert_eq!(false, patterns.is_excluded(&mkpath(".ro"), is_dir));
                assert_eq!(true, patterns.is_excluded(&mkpath("toto.ro"), is_dir));

                // 5.
                assert_eq!(
                    true,
                    patterns.is_excluded(&mkpath("boo/baz/boz/tata"), is_dir)
                );

                assert_eq!(
                    true,
                    patterns.is_excluded(&mkpath("titi/baz/boz/titi"), is_dir)
                );
                assert_eq!(false, patterns.is_excluded(&mkpath("titi/titi"), is_dir));
                assert_eq!(
                    false,
                    patterns.is_excluded(&mkpath("titi/tutu/baz/boz"), is_dir)
                );
                assert_eq!(
                    true,
                    patterns.is_excluded(&mkpath("tutu/baz/boz/titi"), is_dir)
                );
            }
        }
    }
}
