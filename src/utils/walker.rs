use std::{
    env,
    fs::{self, DirEntry},
    io,
    path::{Path, PathBuf},
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
    sync::Arc,
};

use crossbeam::sync::WaitGroup;
use futures::executor::ThreadPool;
use log::{debug, error, info, warn};

use crate::utils::display::Display;
use crate::utils::filters::Filters;
use crate::utils::grep::Grep;
use crate::utils::mapped::Mapped;
use crate::utils::matcher::Matcher;
use crate::utils::patterns::{Patterns, ToPatterns};
use crate::utils::writer::BufferedWriter;

static GIT_IGNORE: &str = ".gitignore";
pub const GIT_DIR: &str = ".git";

#[derive(Clone)]
pub struct Walker {
    tpool: Option<ThreadPool>,
    ignore_patterns: Arc<Patterns>,
    force_ignore_patterns: Arc<Patterns>,
    file_filters: Arc<Filters>,
    grep: Grep,
    matcher: Matcher,
    ignore_symlinks: bool,
    display: Arc<dyn Display>,
    print_file_separator: bool,
    file_separator_printed: Rc<AtomicBool>,
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

    pub fn force_ignore_patterns(mut self, force_ignore_patterns: Patterns) -> WalkerBuilder {
        self.0.force_ignore_patterns = Arc::new(force_ignore_patterns);
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

    pub fn print_file_separator(mut self, print_file_separator: bool) -> WalkerBuilder {
        self.0.print_file_separator = print_file_separator;
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
            force_ignore_patterns: Default::default(),
            file_filters: Default::default(),
            grep,
            matcher,
            ignore_symlinks: false,
            display,
            print_file_separator: false,
            file_separator_printed: Default::default(),
        }
    }

    fn is_ignore_file(&self, entry: &DirEntry) -> bool {
        Some(GIT_IGNORE) == entry.file_name().to_str()
    }

    fn is_excluded(&self, path: &Path, is_dir: bool) -> bool {
        let path = path.to_str().unwrap();
        let skip = self.force_ignore_patterns.is_excluded(path, is_dir);
        if skip {
            info!("Skipping [forced] {:?}", path);
            return true;
        }
        let skip = self.ignore_patterns.is_excluded(path, is_dir);
        if skip {
            info!("Skipping {:?}", path);
        }
        skip
    }

    fn process_gitignore(path: &Path) -> Option<Patterns> {
        let ifile = {
            let mut ifile = path.to_path_buf();
            ifile.push(GIT_IGNORE);
            ifile
        };
        match ifile.to_patterns() {
            Ok(ignore_patterns) => Some(ignore_patterns),
            Err(e) => {
                match e.downcast_ref::<io::Error>() {
                    Some(e) if e.kind() == io::ErrorKind::NotFound => {}
                    _ => error!("Failed to process path '{}': {:?}", ifile.display(), e),
                };
                None
            }
        }
    }

    fn contains_git_dir(path: &Path) -> bool {
        let mut path = path.to_path_buf();
        path.push(GIT_DIR);
        path.exists()
    }

    fn walk_dir(&self, path: &Path, parents: &[PathBuf]) {
        let walker = {
            let mut walker = self.clone();
            if let Some(mut ignore_patterns) = Self::process_gitignore(path) {
                ignore_patterns.extend(&walker.ignore_patterns);
                walker.ignore_patterns = Arc::new(ignore_patterns);
            }
            walker
        };

        let mut to_dive = Vec::new();
        let mut to_grep = Vec::new();

        let mut entries: Vec<_> = fs::read_dir(path)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| !self.is_ignore_file(entry))
            .filter_map(|entry| match entry.file_type() {
                Ok(file_type) => Some((entry.path(), file_type)),
                Err(e) => {
                    error!("Failed to get path '{}' file type: {}", path.display(), e);
                    None
                }
            })
            .filter(|(entry, file_type)| !walker.is_excluded(entry, file_type.is_dir()))
            .collect();
        entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
        for (path, file_type) in entries {
            if file_type.is_file() {
                if !self.file_filters.matches(path.to_str().unwrap()) {
                    continue;
                }
                to_grep.push(path);
            } else {
                to_dive.push((path, file_type));
            }
        }

        let parents = {
            let mut parents = parents.to_owned();
            parents.push(path.to_path_buf());
            parents
        };
        for (entry, file_type) in to_dive {
            walker.walk_with_parents(&entry, Some(file_type), &parents);
        }

        self.grep_many(&to_grep);
    }

    fn grep(grep: Grep, entry: Arc<PathBuf>, matcher: Matcher, display: Arc<dyn Display>) {
        match Mapped::new(&entry) {
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

    fn grep_many(&self, entries: &[PathBuf]) {
        let writer = self.display.writer();
        let mut writers = Vec::with_capacity(entries.len());
        let wg = WaitGroup::new();
        for entry in entries {
            let entry = Arc::new(entry.clone());
            let matcher = self.matcher.clone();
            let writer = Arc::new(BufferedWriter::new());
            let display = self.display.with_writer(writer.clone());
            writers.push(writer);
            if entries.len() < 3 {
                Walker::grep(self.grep.clone(), entry, matcher, display);
                continue;
            }
            match &self.tpool {
                Some(tpool) => {
                    let grep = self.grep.clone();
                    let wg = wg.clone();
                    tpool.spawn_ok(async move {
                        Walker::grep(grep, entry, matcher, display);
                        drop(wg);
                    });
                }
                None => Walker::grep(self.grep.clone(), entry, matcher, display),
            }
        }
        wg.wait();
        for w in writers {
            if self.print_file_separator
                && w.has_some()
                && self.file_separator_printed.swap(true, Ordering::Relaxed)
            {
                self.display.file_separator();
            }
            w.flush(&writer);
        }
    }

    fn canonicalize(&self, orig: &Path, resolved: &Path) -> anyhow::Result<PathBuf> {
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

    fn process_symlink(&self, orig: &Path, resolved: &Path, parents: &[PathBuf]) {
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

    fn walk_with_parents(&self, path: &Path, file_type: Option<fs::FileType>, parents: &[PathBuf]) {
        let file_type = file_type.or_else(|| match fs::symlink_metadata(path) {
            Ok(meta) => Some(meta.file_type()),
            Err(e) => {
                error!("Failed to get path '{}' metadata: {}", path.display(), e);
                None
            }
        });
        let file_type = match file_type {
            Some(file_type) => file_type,
            _ => return,
        };
        if file_type.is_dir() {
            self.walk_dir(path, parents);
        } else if file_type.is_file() {
            Walker::grep(self.grep.clone(), Arc::new(path.to_path_buf()), self.matcher.clone(), self.display.clone());
        } else if file_type.is_symlink() {
            if self.ignore_symlinks {
                info!("Skipping symlink '{}'", path.display());
                return;
            }
            match fs::read_link(path) {
                Ok(resolved) => self.process_symlink(path, &resolved, parents),
                Err(e) => error!("Failed to read link '{}': {}", path.display(), e),
            }
        } else {
            warn!("Unhandled path '{}': {:?}", path.display(), file_type)
        }
    }

    pub fn find_ignore_patterns_in_parents(path: &Path) -> Option<Patterns> {
        if Self::contains_git_dir(path) {
            return None;
        }
        let mut patterns = Vec::new();
        let mut path = path.to_path_buf();
        while path.pop() {
            if let Some(ignore_patterns) = Self::process_gitignore(&path) {
                debug!("Found .gitignore in {}", path.display());
                patterns.push(ignore_patterns);
            }
            if Self::contains_git_dir(&path) {
                break;
            }
        }
        if patterns.is_empty() {
            return None;
        }
        let mut ignore_patterns = Patterns::default();
        for pattern in patterns {
            ignore_patterns.extend(&pattern);
        }
        Some(ignore_patterns)
    }

    pub fn walk(&self, path: &Path) {
        self.walk_with_parents(path, None, &[]);
    }
}
