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
use crate::utils::lines::Zero;
use crate::utils::mapped::Mapped;
use crate::utils::matcher::Matcher;
use crate::utils::patterns::{Patterns, ToPatterns};
use crate::utils::timing;
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
        WalkerBuilder(Walker::new(grep, matcher, display))
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
        timing::time("walk.exclude", || {
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
        })
    }

    fn process_gitignore(path: &Path) -> Option<Patterns> {
        let ifile = {
            let mut ifile = path.to_path_buf();
            ifile.push(GIT_IGNORE);
            ifile
        };
        match timing::time("walk.gitignore", || ifile.to_patterns()) {
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
        let mut walker = self.clone();

        let mut to_dive = Vec::new();
        let mut to_grep = Vec::new();

        let entries: Vec<_> = timing::time("walk.read_dir", || {
            fs::read_dir(path)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| match entry.file_type() {
                    Ok(file_type) => {
                        let is_ignore_file = self.is_ignore_file(&entry);
                        Some((entry.path(), file_type, is_ignore_file))
                    }
                    Err(e) => {
                        error!("Failed to get path '{}' file type: {}", path.display(), e);
                        None
                    }
                })
                .collect()
        });
        if let Some(ignore_path) = entries
            .iter()
            .find_map(|(entry, _, is_ignore_file)| is_ignore_file.then_some(entry))
        {
            match timing::time("walk.gitignore", || ignore_path.to_patterns()) {
                Ok(mut ignore_patterns) => {
                    ignore_patterns.extend(&walker.ignore_patterns);
                    walker.ignore_patterns = Arc::new(ignore_patterns);
                }
                Err(e) => error!(
                    "Failed to process path '{}': {:?}",
                    ignore_path.display(),
                    e
                ),
            }
        }
        let mut entries: Vec<_> = entries
            .into_iter()
            .filter(|(_, _, is_ignore_file)| !is_ignore_file)
            .filter(|(entry, file_type, _)| !walker.is_excluded(entry, file_type.is_dir()))
            .map(|(entry, file_type, _)| (entry, file_type))
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
        let len = fs::metadata(entry.as_path())
            .ok()
            .map(|meta| meta.len() as usize);
        if matches!(len, Some(0)) {
            (grep)(Arc::new(Zero::new((*entry).clone())), matcher, display);
            return;
        }
        match len.and_then(|len| Mapped::new(&entry, len).ok()) {
            Some(mapped) => {
                if content_inspector::inspect(&mapped).is_binary() {
                    debug!("Skipping binary file '{}'", entry.display());
                    return;
                }
                (grep)(Arc::new(mapped), matcher, display);
            }
            None => {
                warn!("Failed to map file '{}'", entry.display());
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
        env::set_current_dir(parent)?;
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
            Walker::grep(
                self.grep.clone(),
                Arc::new(path.to_path_buf()),
                self.matcher.clone(),
                self.display.clone(),
            );
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

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::utils::display::DisplayTerminal;
    use crate::utils::display::{Format, PathFormat};
    use crate::utils::matcher::Match;
    use crate::utils::writer::Writer;

    #[derive(Clone)]
    struct TestWriter {
        writes: Arc<Mutex<Vec<String>>>,
    }

    impl TestWriter {
        fn new() -> Self {
            Self {
                writes: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn lines(&self) -> Vec<String> {
            self.writes.lock().unwrap().clone()
        }
    }

    impl Writer for TestWriter {
        fn write(&self, content: &str) {
            self.writes.lock().unwrap().push(content.to_owned());
        }
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let mut path = std::env::temp_dir();
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            path.push(format!("tgrep-walker-test-{}", unique));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn write(&self, relative: &str, contents: &[u8]) {
            let path = self.path.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, contents).unwrap();
        }

        fn mkdir(&self, relative: &str) {
            fs::create_dir_all(self.path.join(relative)).unwrap();
        }

        #[cfg(unix)]
        fn symlink_dir(&self, target: &Path, link: &str) {
            symlink(target, self.path.join(link)).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn matcher() -> Matcher {
        Arc::new(Box::new(|_, _| Some(vec![Match::new(0, 0)])))
    }

    fn display(writer: Arc<dyn Writer>) -> Arc<dyn Display> {
        let path_format: PathFormat = Arc::new(Box::new(|path: &Path| {
            path.file_name().unwrap().to_string_lossy().into_owned()
        }));
        Arc::new(DisplayTerminal::new(
            120,
            Format::PathOnly { colour: false },
            path_format,
            writer,
        ))
    }

    fn grep_recorder() -> Grep {
        Arc::new(Box::new(|reader, _, display| {
            display.display(reader.path(), None);
        }))
    }

    #[test]
    fn finds_parent_gitignore_until_git_dir() {
        let temp = TempDir::new();
        temp.mkdir(".git");
        temp.write(".gitignore", b"root-ignored.txt\n");
        temp.mkdir("nested/deep");
        temp.write("nested/.gitignore", b"nested-ignored.txt\n");

        let patterns = Walker::find_ignore_patterns_in_parents(&temp.path().join("nested/deep"))
            .expect("expected parent ignore patterns");

        let root_ignored = temp.path().join("root-ignored.txt");
        let nested_ignored = temp.path().join("nested/nested-ignored.txt");
        let outside = temp.path().join("nested/deep/visible.txt");

        assert!(patterns.is_excluded(root_ignored.to_str().unwrap(), false));
        assert!(patterns.is_excluded(nested_ignored.to_str().unwrap(), false));
        assert!(!patterns.is_excluded(outside.to_str().unwrap(), false));
    }

    #[test]
    fn does_not_search_beyond_repository_root_for_parent_gitignores() {
        let outer = TempDir::new();
        outer.write(".gitignore", b"outside.txt\n");
        outer.mkdir("repo/.git");
        outer.mkdir("repo/nested");

        let patterns = Walker::find_ignore_patterns_in_parents(&outer.path().join("repo/nested"));

        let outside = outer.path().join("outside.txt");
        assert!(
            patterns.is_none()
                || !patterns
                    .unwrap()
                    .is_excluded(outside.to_str().unwrap(), false)
        );
    }

    #[test]
    fn walk_honors_gitignore_and_file_filters() {
        let temp = TempDir::new();
        temp.write(".gitignore", b"ignored.txt\n");
        temp.write("visible.rs", b"fn main() {}\n");
        temp.write("ignored.txt", b"secret\n");
        temp.write("notes.md", b"# notes\n");

        let writer = TestWriter::new();
        let walker = WalkerBuilder::new(
            grep_recorder(),
            matcher(),
            display(Arc::new(writer.clone())),
        )
        .ignore_patterns(Patterns::new(temp.path().to_str().unwrap(), &[]))
        .force_ignore_patterns(Patterns::new(temp.path().to_str().unwrap(), &[]))
        .file_filters(Filters::new(&["*.rs".to_string()]).unwrap())
        .build();

        walker.walk(temp.path());

        assert_eq!(vec!["visible.rs"], writer.lines());
    }

    #[test]
    fn force_ignore_patterns_override_walk_results() {
        let temp = TempDir::new();
        temp.write("visible.txt", b"ok\n");
        temp.write("forced.txt", b"skip\n");

        let writer = TestWriter::new();
        let walker = WalkerBuilder::new(
            grep_recorder(),
            matcher(),
            display(Arc::new(writer.clone())),
        )
        .ignore_patterns(Patterns::new(temp.path().to_str().unwrap(), &[]))
        .force_ignore_patterns(Patterns::new(
            temp.path().to_str().unwrap(),
            &["forced.txt".to_string()],
        ))
        .file_filters(Filters::new(&["*".to_string()]).unwrap())
        .build();

        walker.walk(temp.path());

        assert_eq!(vec!["visible.txt"], writer.lines());
    }

    #[cfg(unix)]
    #[test]
    fn walk_skips_symlinks_when_configured() {
        let temp = TempDir::new();
        let external = TempDir::new();
        external.write("linked.txt", b"external\n");
        temp.symlink_dir(external.path(), "external-link");

        let writer = TestWriter::new();
        let walker = WalkerBuilder::new(
            grep_recorder(),
            matcher(),
            display(Arc::new(writer.clone())),
        )
        .ignore_patterns(Patterns::new(temp.path().to_str().unwrap(), &[]))
        .force_ignore_patterns(Patterns::new(temp.path().to_str().unwrap(), &[]))
        .file_filters(Filters::new(&["*".to_string()]).unwrap())
        .ignore_symlinks(true)
        .build();

        walker.walk(temp.path());

        assert!(writer.lines().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn walk_follows_external_directory_symlinks() {
        let temp = TempDir::new();
        let external = TempDir::new();
        external.write("linked.txt", b"external\n");
        temp.symlink_dir(external.path(), "external-link");

        let writer = TestWriter::new();
        let walker = WalkerBuilder::new(
            grep_recorder(),
            matcher(),
            display(Arc::new(writer.clone())),
        )
        .ignore_patterns(Patterns::new(temp.path().to_str().unwrap(), &[]))
        .force_ignore_patterns(Patterns::new(temp.path().to_str().unwrap(), &[]))
        .file_filters(Filters::new(&["*".to_string()]).unwrap())
        .build();

        walker.walk(temp.path());

        assert_eq!(vec!["linked.txt"], writer.lines());
    }
}
