use std::{
    env,
    fs::{self, DirEntry},
    path::PathBuf,
    sync::Arc,
};

use crossbeam::sync::WaitGroup;
use futures::executor::ThreadPool;
use log::{debug, error, info, warn};

use crate::utils::display::{Display, DisplayContext};
use crate::utils::filters::Filters;
use crate::utils::lines::LinesReader;
use crate::utils::matcher::Matcher;
use crate::utils::patterns::{Patterns, ToPatterns};

#[derive(Clone)]
pub struct Walker {
    tpool: Option<ThreadPool>,
    ignore_patterns: Patterns,
    ignore_files: Vec<String>,
    file_filters: Filters,
    matcher: Matcher,
    max_matches_per_file: usize,
    ignore_symlinks: bool,
    display: Arc<dyn Display>,
}

pub struct WalkerBuilder(Walker);

impl WalkerBuilder {
    pub fn new(matcher: Matcher, display: Arc<dyn Display>) -> Self {
        WalkerBuilder {
            0: Walker::new(matcher, display),
        }
    }

    pub fn thread_pool(mut self, tpool: ThreadPool) -> WalkerBuilder {
        self.0.tpool = Some(tpool);
        self
    }

    pub fn ignore_patterns(mut self, ignore_patterns: Patterns) -> WalkerBuilder {
        self.0.ignore_patterns = ignore_patterns;
        self
    }

    pub fn ignore_files(mut self, ignore_files: Vec<String>) -> WalkerBuilder {
        self.0.ignore_files = ignore_files;
        self
    }

    pub fn file_filters(mut self, file_filters: Filters) -> WalkerBuilder {
        self.0.file_filters = file_filters;
        self
    }

    pub fn max_matches_per_file(mut self, max_matches_per_file: usize) -> WalkerBuilder {
        self.0.max_matches_per_file = max_matches_per_file;
        self
    }

    pub fn ignore_symlinks(mut self, ignore_symlinks: bool) -> WalkerBuilder {
        self.0.ignore_symlinks = ignore_symlinks;
        self
    }

    pub fn build(self) -> Walker {
        self.0
    }
}

impl Walker {
    pub fn new(matcher: Matcher, display: Arc<dyn Display>) -> Self {
        Walker {
            tpool: None,
            ignore_patterns: Default::default(),
            ignore_files: vec![],
            file_filters: Default::default(),
            matcher,
            max_matches_per_file: usize::MAX,
            ignore_symlinks: false,
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

    pub fn grep<T: LinesReader>(
        reader: &T,
        matcher: Matcher,
        max: usize,
        display: Arc<dyn Display>,
    ) {
        let mut matches = 0;
        match reader.lines() {
            Ok(lines) => {
                for (lno, line) in lines.enumerate() {
                    match line {
                        Ok(line) => {
                            if let Some(needle) = matcher(&line) {
                                display.display(
                                    reader.path(),
                                    Some(DisplayContext::new(lno, &line, needle)),
                                );
                                matches += 1;
                                if matches >= max {
                                    return;
                                }
                            }
                        }
                        Err(e) => match e.kind() {
                            std::io::ErrorKind::InvalidData => {
                                // Likely binary file
                                debug!("Failed to read '{}': {}", reader.path().display(), e);
                                return;
                            }
                            _ => {
                                warn!("Failed to read '{}': {}", reader.path().display(), e);
                                return;
                            }
                        },
                    }
                }
            }
            Err(e) => error!("Failed to read '{}': {}", reader.path().display(), e),
        }
    }

    fn walk_with_parents(&self, path: &PathBuf, parents: &[PathBuf]) {
        let meta = fs::symlink_metadata(path.as_path());
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
            let walker = {
                let mut walker = self.clone();
                walker.ignore_patterns = ignore_patterns.clone();
                walker
            };
            let wg = WaitGroup::new();
            for entry in entries
                .iter()
                .filter(|entry| !self.is_excluded(&ignore_patterns, entry))
            {
                match entry.metadata() {
                    Ok(meta) => {
                        let file_type = meta.file_type();
                        if file_type.is_file() {
                            let path = entry.path();
                            if !self.file_filters.matches(path.to_str().unwrap()) {
                                continue;
                            }
                            let matcher = self.matcher.clone();
                            let max = self.max_matches_per_file;
                            let display = self.display.clone();
                            match &self.tpool {
                                Some(tpool) => {
                                    let wg = wg.clone();
                                    tpool.spawn_ok(async move {
                                        Walker::grep(&path, matcher, max, display);
                                        drop(wg);
                                    });
                                }
                                None => Walker::grep(&path, matcher, max, display),
                            }
                        } else {
                            walker.walk_with_parents(&entry.path(), &{
                                let mut parents = parents.to_owned();
                                parents.push(path.clone());
                                parents
                            });
                        }
                    }
                    Err(e) => error!("Failed to get path '{}' metadata: {}", path.display(), e),
                }
            }
            wg.wait();
        } else if file_type.is_file() {
            Walker::grep(
                path,
                self.matcher.clone(),
                self.max_matches_per_file,
                self.display.clone(),
            );
        } else if file_type.is_symlink() {
            if self.ignore_symlinks {
                info!("Skipping symlink '{}'", path.display());
                return;
            }
            let orig = path;
            match fs::read_link(path.as_path()) {
                Ok(lpath) => {
                    let canonicalize = || {
                        let cwd = env::current_dir()?;
                        let parent = path
                            .parent()
                            .ok_or_else(|| anyhow::Error::msg("No parent"))?;
                        env::set_current_dir(&parent)?;
                        let path = lpath.canonicalize().map_err(|e| {
                            anyhow::Error::new(e).context(format!("cwd {}", parent.display()))
                        });
                        env::set_current_dir(&cwd)?;
                        path
                    };
                    let path = canonicalize();
                    if let Err(e) = path {
                        error!("Failed to canonicalize '{}': {}", lpath.display(), e);
                        return;
                    }
                    let path = path.unwrap();
                    if let Some(level) = parents.iter().position(|parent| *parent == path) {
                        error!(
                            "Symlink '{}' -> '{}' (dereferenced to '{}') loop detected at level {}",
                            orig.display(),
                            lpath.display(),
                            path.display(),
                            level,
                        );
                        return;
                    }
                    if parents.iter().any(|parent| path.starts_with(parent)) {
                        info!(
                            "Skipping symlink '{}' -> '{}' (dereferenced to '{}')",
                            orig.display(),
                            lpath.display(),
                            path.display(),
                        );
                        return;
                    }
                    self.walk_with_parents(&path, &{
                        let mut parents = parents.to_owned();
                        parents.push(path.clone());
                        parents
                    });
                }
                Err(e) => error!("Failed to read link '{}': {}", path.display(), e),
            }
        } else {
            warn!("Unhandled path '{}': {:?}", path.display(), file_type)
        }
    }

    pub fn walk(&self, path: &PathBuf) {
        self.walk_with_parents(path, &[]);
    }
}
