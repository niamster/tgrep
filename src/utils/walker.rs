use std::{
    fs::{self, DirEntry},
    path::PathBuf,
    sync::Arc,
};

use crossbeam::sync::WaitGroup;
use futures::executor::ThreadPool;
use quicli::prelude::*;
use regex::Regex;

use crate::utils::display::Display;
use crate::utils::lines::ToLines;
use crate::utils::patterns::{Patterns, ToPatterns};

#[derive(Clone)]
pub struct Walker {
    ignore_patterns: Patterns,
    ignore_files: Box<Vec<String>>,
    regexp: Box<Regex>,
    display: Arc<Box<dyn Display>>,
}

impl Walker {
    pub fn new(
        ignore_patterns: Patterns,
        ignore_files: Box<Vec<String>>,
        regexp: Box<Regex>,
        display: Arc<Box<dyn Display>>,
    ) -> Self {
        Walker {
            ignore_patterns,
            ignore_files,
            regexp,
            display,
        }
    }

    fn is_ignore_file(&self, entry: &DirEntry) -> bool {
        let file_name = entry.file_name().to_str().unwrap().to_string();
        self.ignore_files.contains(&file_name)
    }

    fn is_excluded(&self, patterns: &Patterns, entry: &DirEntry) -> bool {
        let path = entry.path();
        let is_dir = path.is_dir();
        let path = path.to_str().unwrap();
        let is_excluded = patterns.is_excluded(&path, is_dir);
        if is_excluded {
            info!("Skipping {:?}", entry.path());
        }
        is_excluded
    }

    fn grep(path: Box<PathBuf>, regexp: Box<Regex>, display: Arc<Box<dyn Display>>) {
        match path.to_lines() {
            Ok(lines) => {
                for (lno, line) in lines.enumerate() {
                    match line {
                        Ok(line) => {
                            if let Some(needle) = regexp.find(&line) {
                                display.display(&path, lno, &line, &needle)
                            }
                        }
                        Err(e) => match e.kind() {
                            std::io::ErrorKind::InvalidData => {
                                // Likely binary file
                                debug!("Failed to read '{}': {}", path.display(), e);
                                return;
                            }
                            _ => {
                                warn!("Failed to read '{}': {}", path.display(), e);
                                return;
                            }
                        },
                    }
                }
            }
            Err(e) => error!("Failed to read '{}': {}", path.display(), e),
        }
    }

    pub fn walk(&self, tpool: &ThreadPool, path: &PathBuf) {
        let meta = fs::metadata(path.as_path());
        if let Err(e) = meta {
            error!("Failed to get path '{}' metadata: {}", path.display(), e);
            return;
        }
        let file_type = meta.unwrap().file_type();
        if file_type.is_dir() {
            let (ignore_files, entries): (Vec<_>, Vec<_>) = fs::read_dir(path)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .partition(|entry| self.is_ignore_file(entry));
            let ignore_files: Vec<_> = ignore_files.iter().map(|entry| entry.path()).collect();
            let mut ignore_patterns = ignore_files.to_patterns();
            ignore_patterns.extend(&self.ignore_patterns);
            let walker = Walker::new(
                ignore_patterns.clone(),
                self.ignore_files.clone(),
                self.regexp.clone(),
                self.display.clone(),
            );
            let wg = WaitGroup::new();
            for entry in entries
                .into_iter()
                .filter(|entry| !self.is_excluded(&ignore_patterns, entry))
            {
                match entry.metadata() {
                    Ok(meta) => {
                        let file_type = meta.file_type();
                        if file_type.is_file() {
                            let regexp = self.regexp.clone();
                            let display = self.display.clone();
                            let wg = wg.clone();
                            tpool.spawn_ok(async move {
                                Walker::grep(Box::new(entry.path()), regexp, display);
                                drop(wg);
                            });
                        } else {
                            walker.walk(tpool, &entry.path());
                        }
                    }
                    Err(e) => error!("Failed to get path '{}' metadata: {}", path.display(), e),
                }
            }
            wg.wait();
        } else if file_type.is_file() {
            Walker::grep(
                Box::new(path.to_path_buf()),
                self.regexp.clone(),
                self.display.clone(),
            );
        } else if file_type.is_symlink() {
            error!(
                "Symlinks are not (yet) supported, skipping '{}'",
                path.display()
            );
        } else {
            warn!("Unhandled path '{}': {:?}", path.display(), file_type)
        }
    }
}
