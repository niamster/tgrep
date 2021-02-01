use std::{
    fs::{self, DirEntry},
    path::PathBuf,
};

use glob::Pattern;
use quicli::prelude::*;
use regex::Regex;

use crate::utils::display::Display;
use crate::utils::lines::ToLines;
use crate::utils::patterns::ToPatterns;

pub struct Walker<'a> {
    ignore_patterns: Vec<Pattern>,
    ignore_files: &'a Vec<String>,
    regexp: &'a Regex,
    display: &'a dyn Display,
}

impl<'a> Walker<'a> {
    pub fn new(
        ignore_patterns: Vec<Pattern>,
        ignore_files: &'a Vec<String>,
        regexp: &'a Regex,
        display: &'a dyn Display,
    ) -> Self {
        Walker {
            ignore_patterns,
            ignore_files,
            regexp,
            display,
        }
    }

    fn must_ignore(
        &self,
        ignore_patterns: &Vec<Pattern>,
        root: &PathBuf,
        entry: &DirEntry,
    ) -> bool {
        let path = entry.path();
        let is_dir = path.is_dir();
        let path = path.strip_prefix(root).unwrap();
        let path = path.to_str().unwrap();
        let mut paths = vec![path.to_string()];
        if is_dir {
            // FIXME: extra ugly stuff to w/a globs and avoid entering dir
            paths.push(path.to_owned() + "/");
            paths.push(path.to_owned() + "/*");
        }
        for pattern in ignore_patterns {
            for path in &paths {
                if pattern.matches(path.as_str()) {
                    debug!("Skipping {:?}", entry.path());
                    return true;
                }
            }
        }
        false
    }

    fn is_ignore_file(&self, entry: &DirEntry) -> bool {
        let file_name = entry.file_name().to_str().unwrap().to_string();
        self.ignore_files.contains(&file_name)
    }

    fn walk_with_root(&self, root: &PathBuf, path: &PathBuf) {
        let meta = fs::metadata(path.as_path());
        if let Err(e) = meta {
            error!("Failed to get path '{}' metadata: {}", path.display(), e);
            return;
        }
        let file_type = meta.unwrap().file_type();
        if file_type.is_dir() {
            let (ignore_files, entries): (Vec<DirEntry>, Vec<DirEntry>) = fs::read_dir(path)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .partition(|entry| self.is_ignore_file(entry));
            let ignore_files: Vec<PathBuf> =
                ignore_files.iter().map(|entry| entry.path()).collect();
            let (root_ignore_patterns, local_ignore_patterns) = ignore_files.to_patterns();
            let local_ignore_patterns = {
                let mut patterns = self.ignore_patterns.clone();
                patterns.extend(local_ignore_patterns);
                patterns.dedup();
                patterns
            };
            let walker = Walker::new(
                local_ignore_patterns.clone(),
                self.ignore_files,
                self.regexp,
                self.display,
            );
            for entry in entries
                .iter()
                .filter(|entry| !self.must_ignore(&root_ignore_patterns, root, entry))
                .filter(|entry| !self.must_ignore(&local_ignore_patterns, root, entry))
            {
                walker.walk_with_root(root, &entry.path());
            }
        } else if file_type.is_file() {
            match path.to_lines() {
                Ok(lines) => {
                    let mut lno: u32 = 0;
                    for line in lines {
                        match line {
                            Ok(line) => {
                                if let Some(needle) = self.regexp.find(line.as_str()) {
                                    self.display.display(&path, lno, &line, &needle)
                                }
                            }
                            Err(e) => {
                                warn!("Failed to read '{}': {}", path.display(), e);
                                return;
                            }
                        }
                        lno += 1;
                    }
                }
                Err(e) => error!("Failed to read '{}': {}", path.display(), e),
            }
        } else if file_type.is_symlink() {
            error!(
                "Symlinks are not (yet) supported, skipping '{}'",
                path.display()
            );
        } else {
            warn!("Unhandled path '{}': {:?}", path.display(), file_type)
        }
    }

    pub fn walk(&self, path: &PathBuf) {
        self.walk_with_root(path, path)
    }
}
