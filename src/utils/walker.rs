use std::{
    collections::BTreeMap,
    env,
    fs::{self, DirEntry},
    path::PathBuf,
    sync::Arc,
};

use crossbeam::sync::WaitGroup;
use futures::executor::ThreadPool;
use log::{debug, error, info, warn};
use rayon::prelude::*;

use crate::utils::display::Display;
use crate::utils::filters::Filters;
use crate::utils::grep::Grep;
use crate::utils::lines::Zero;
use crate::utils::mapped::Mapped;
use crate::utils::matcher::Matcher;
use crate::utils::patterns::{Patterns, ToPatterns};
use crate::utils::writer::BufferedWriter;

static GIT_IGNORE: &str = ".gitignore";

#[derive(Clone)]
pub struct Walker {
    tpool: Option<ThreadPool>,
    ignore_patterns: Arc<Patterns>,
    file_filters: Arc<Filters>,
    grep: Grep,
    matcher: Matcher,
    ignore_symlinks: bool,
    display: Arc<dyn Display>,
}

pub struct WalkerBuilder(Walker);

impl WalkerBuilder {
    pub fn new(grep: Grep, matcher: Matcher, display: Arc<dyn Display>) -> Self {
        WalkerBuilder {
            0: Walker::new(grep, matcher, display),
        }
    }

    pub fn thread_pool(mut self, tpool: ThreadPool) -> WalkerBuilder {
        self.0.tpool = Some(tpool);
        self
    }

    pub fn ignore_patterns(mut self, ignore_patterns: Patterns) -> WalkerBuilder {
        self.0.ignore_patterns = Arc::new(ignore_patterns);
        self
    }

    pub fn file_filters(mut self, file_filters: Filters) -> WalkerBuilder {
        self.0.file_filters = Arc::new(file_filters);
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
    pub fn new(grep: Grep, matcher: Matcher, display: Arc<dyn Display>) -> Self {
        Walker {
            tpool: None,
            ignore_patterns: Default::default(),
            file_filters: Default::default(),
            grep,
            matcher,
            ignore_symlinks: false,
            display,
        }
    }

    fn is_ignore_file(&self, entry: &DirEntry) -> bool {
        Some(GIT_IGNORE) == entry.file_name().to_str()
    }

    fn is_excluded(&self, entry: &DirEntry) -> bool {
        let path = entry.path();
        let is_dir = path.is_dir();
        let path = path.to_str().unwrap();
        let skip = self.ignore_patterns.is_excluded(&path, is_dir);
        if skip {
            info!("Skipping {:?}", entry.path());
        }
        skip
    }

    fn walk_dir(&self, path: &PathBuf, parents: &[PathBuf]) {
        let (ignore_files, entries): (Vec<_>, Vec<_>) = fs::read_dir(path)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .partition(|entry| self.is_ignore_file(entry));

        let walker = {
            let mut walker = self.clone();
            if !ignore_files.is_empty() {
                let ignore_files: Vec<_> = ignore_files.iter().map(|entry| entry.path()).collect();
                let mut ignore_patterns = ignore_files.to_patterns();
                ignore_patterns.extend(&walker.ignore_patterns);
                walker.ignore_patterns = Arc::new(ignore_patterns);
            }
            walker
        };

        let mut to_dive = BTreeMap::new();
        let mut to_grep = Vec::new();

        let entries: Vec<_> = entries
            .par_iter()
            .filter(|entry| !walker.is_excluded(entry))
            .map(|entry| (entry.path(), entry.metadata()))
            .collect();
        for (path, meta) in entries {
            match meta {
                Ok(meta) => {
                    let file_type = meta.file_type();
                    if file_type.is_file() {
                        if !self.file_filters.matches(path.to_str().unwrap()) {
                            continue;
                        }
                        to_grep.push((path, meta.len() as usize));
                    } else {
                        to_dive.insert(path, meta);
                    }
                }
                Err(e) => error!("Failed to get path '{}' metadata: {}", path.display(), e),
            }
        }

        let parents = {
            let mut parents = parents.to_owned();
            parents.push(path.clone());
            parents
        };
        for (entry, meta) in to_dive {
            walker.walk_with_parents(&entry, Some(meta), &parents);
        }

        self.grep_many(&to_grep);
    }

    fn grep(
        grep: Grep,
        entry: Arc<PathBuf>,
        len: usize,
        matcher: Matcher,
        display: Arc<dyn Display>,
    ) {
        match Mapped::new(&entry, len) {
            Ok(mapped) => {
                if content_inspector::inspect(&*mapped).is_binary() {
                    debug!("Skipping binary file '{}'", entry.display());
                    return;
                }
                (grep)(Arc::new(mapped), matcher, display);
            }
            Err(e) => {
                warn!("Failed to map file '{}': {}", entry.display(), e);
                (grep)(entry, matcher, display);
            }
        }
    }

    fn grep_many(&self, entries: &[(PathBuf, usize)]) {
        let writer = self.display.writer();
        let mut writers = BTreeMap::new();
        let wg = WaitGroup::new();
        for (entry, len) in entries {
            let entry = Arc::new(entry.clone());
            let matcher = self.matcher.clone();
            let writer = Arc::new(BufferedWriter::new());
            let display = self.display.with_writer(writer.clone());
            writers.insert(entry.clone(), writer);
            let len = *len;
            if len == 0 {
                (self.grep)(Arc::new(Zero::new((*entry).clone())), matcher, display);
                continue;
            }
            match &self.tpool {
                Some(tpool) => {
                    let grep = self.grep;
                    let wg = wg.clone();
                    tpool.spawn_ok(async move {
                        Walker::grep(grep, entry, len, matcher, display);
                        drop(wg);
                    });
                }
                None => Walker::grep(self.grep, entry, len, matcher, display),
            }
        }
        wg.wait();
        for (_, w) in writers {
            w.flush(&writer);
        }
    }

    fn canonicalize(&self, orig: &PathBuf, resolved: &PathBuf) -> anyhow::Result<PathBuf> {
        let cwd = env::current_dir()?;
        let parent = orig
            .parent()
            .ok_or_else(|| anyhow::Error::msg("no parent"))?;
        env::set_current_dir(&parent)?;
        let path = resolved
            .canonicalize()
            .map_err(|e| anyhow::Error::new(e).context(format!("cwd {}", parent.display())));
        env::set_current_dir(&cwd)?;
        path
    }

    fn process_symlink(&self, orig: &PathBuf, resolved: &PathBuf, parents: &[PathBuf]) {
        let path = self.canonicalize(orig, resolved);
        if let Err(e) = path {
            error!("Failed to canonicalize '{}': {}", resolved.display(), e);
            return;
        }
        let path = path.unwrap();
        if let Some(level) = parents.iter().position(|parent| *parent == path) {
            error!(
                "Symlink '{}' -> '{}' (dereferenced to '{}') loop detected at level {}",
                orig.display(),
                resolved.display(),
                path.display(),
                level,
            );
            return;
        }
        if parents.iter().any(|parent| path.starts_with(parent)) {
            info!(
                "Skipping symlink '{}' -> '{}' (dereferenced to '{}')",
                orig.display(),
                resolved.display(),
                path.display(),
            );
            return;
        }
        self.walk_with_parents(&path, None, &{
            let mut parents = parents.to_owned();
            parents.push(path.clone());
            parents
        });
    }

    fn walk_with_parents(&self, path: &PathBuf, meta: Option<fs::Metadata>, parents: &[PathBuf]) {
        let meta = meta.or_else(|| match fs::symlink_metadata(path.as_path()) {
            Ok(meta) => Some(meta),
            Err(e) => {
                error!("Failed to get path '{}' metadata: {}", path.display(), e);
                None
            }
        });
        let meta = match meta {
            Some(meta) => meta,
            _ => return,
        };
        let file_type = meta.file_type();
        if file_type.is_dir() {
            self.walk_dir(path, parents);
        } else if file_type.is_file() {
            Walker::grep(
                self.grep,
                Arc::new(path.clone()),
                meta.len() as usize,
                self.matcher.clone(),
                self.display.clone(),
            );
        } else if file_type.is_symlink() {
            if self.ignore_symlinks {
                info!("Skipping symlink '{}'", path.display());
                return;
            }
            match fs::read_link(path.as_path()) {
                Ok(resolved) => self.process_symlink(path, &resolved, parents),
                Err(e) => error!("Failed to read link '{}': {}", path.display(), e),
            }
        } else {
            warn!("Unhandled path '{}': {:?}", path.display(), file_type)
        }
    }

    pub fn walk(&self, path: &PathBuf) {
        self.walk_with_parents(path, None, &[]);
    }
}
