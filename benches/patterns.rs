use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use criterion::{criterion_group, criterion_main, Criterion};
use regex::Regex;

use tgrep::utils::display::{Display, DisplayTerminal, Format, PathFormat};
use tgrep::utils::filters::Filters;
use tgrep::utils::grep::{self, Grep};
use tgrep::utils::lines::Zero;
use tgrep::utils::mapped::Mapped;
use tgrep::utils::matcher::{Match, Matcher, MatcherOptions};
use tgrep::utils::patterns::Patterns;
use tgrep::utils::walker::WalkerBuilder;
use tgrep::utils::writer::Writer;

fn unique_path(prefix: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    path.push(format!("{}-{}", prefix, unique));
    path
}

struct TempTree {
    path: PathBuf,
}

impl TempTree {
    fn new(prefix: &str) -> Self {
        let path = unique_path(prefix);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn collect_files(&self) -> Vec<PathBuf> {
        fn walk(path: &Path, files: &mut Vec<PathBuf>) {
            let entries = fs::read_dir(path).unwrap();
            for entry in entries {
                let entry = entry.unwrap();
                let path = entry.path();
                let file_type = entry.file_type().unwrap();
                if file_type.is_dir() {
                    walk(&path, files);
                } else if file_type.is_file() {
                    files.push(path);
                }
            }
        }

        let mut files = Vec::new();
        walk(&self.path, &mut files);
        files.sort();
        files
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Clone, Default)]
struct NullWriter;

impl Writer for NullWriter {
    fn write(&self, _content: &str) {}
}

fn build_matcher(needle: &str) -> Matcher {
    let regexp = Regex::new(needle).unwrap();
    Arc::new(Box::new(move |line: &str, options| -> Option<Vec<Match>> {
        match options {
            MatcherOptions::Fuzzy => regexp
                .shortest_match(line)
                .map(|pos| vec![Match::new(0, pos)]),
            MatcherOptions::Exact(max) => {
                let mut matches = Vec::new();
                for (idx, hit) in regexp.find_iter(line).enumerate() {
                    matches.push(Match::new(hit.start(), hit.end()));
                    if idx + 1 == max {
                        break;
                    }
                }
                if matches.is_empty() {
                    None
                } else {
                    Some(matches)
                }
            }
        }
    }))
}

fn build_display(root: &Path) -> Arc<dyn Display> {
    let root = root.to_path_buf();
    let path_format: PathFormat = Arc::new(Box::new(move |path: &Path| {
        path.strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }));
    Arc::new(DisplayTerminal::new(
        usize::MAX,
        Format::Rich {
            colour: false,
            match_only: false,
            no_path: false,
            no_lno: false,
        },
        path_format,
        Arc::new(NullWriter),
    ))
}

fn run_grep_on_files(files: &[PathBuf], matcher: Matcher, display: Arc<dyn Display>, grep: Grep) {
    for path in files {
        if let Some(mapped) = Mapped::open(path).unwrap() {
            if content_inspector::inspect(&mapped).is_binary() {
                continue;
            }
            grep(Arc::new(mapped), matcher.clone(), display.clone());
        } else {
            grep(
                Arc::new(Zero::new(path.to_path_buf())),
                matcher.clone(),
                display.clone(),
            );
        }
    }
}

fn bench_tree_sparse() -> TempTree {
    let tree = TempTree::new("tgrep-bench-sparse");
    tree.write(".gitignore", "ignored/\n");
    for dir_idx in 0..12 {
        for file_idx in 0..40 {
            let name = format!("dir-{dir_idx:02}/file-{file_idx:03}.txt");
            if file_idx % 17 == 0 {
                tree.write(&name, "alpha\nbeta\nneedle\ngamma\n");
            } else {
                tree.write(&name, "alpha\nbeta\ngamma\ndelta\n");
            }
        }
    }
    for file_idx in 0..80 {
        let name = format!("ignored/file-{file_idx:03}.txt");
        tree.write(&name, "needle\n");
    }
    tree
}

fn bench_tree_dense() -> TempTree {
    let tree = TempTree::new("tgrep-bench-dense");
    for dir_idx in 0..8 {
        for file_idx in 0..24 {
            let name = format!("dense-{dir_idx:02}/file-{file_idx:03}.txt");
            let mut body = String::new();
            for line_idx in 0..200 {
                let line = if line_idx % 3 == 0 {
                    "needle needle needle"
                } else {
                    "some ordinary text"
                };
                body.push_str(line);
                body.push('\n');
            }
            tree.write(&name, &body);
        }
    }
    tree
}

fn patterns_bench(c: &mut Criterion) {
    let _ = env_logger::builder().is_test(true).try_init();
    let patterns = Patterns::new("/", &["foo/bar/**/qux/xyz".to_string()]);
    c.bench_function("patterns_double_star_match", |b| {
        b.iter(|| {
            patterns.is_excluded(black_box("foo/bar/zoo/too/qux/xyz"), false);
        })
    });
}

fn traversal_bench(c: &mut Criterion) {
    let tree = bench_tree_sparse();
    let matcher = build_matcher("needle");
    let display = build_display(tree.path());
    let grep: Grep = Arc::new(Box::new(|_, _, _| {}));
    let file_filters = Filters::new(&["*.unlikely".to_string()]).unwrap();
    let ignore_patterns = Patterns::new(tree.path().to_str().unwrap(), &[]);
    let force_ignore_patterns = Patterns::new(tree.path().to_str().unwrap(), &[]);

    c.bench_function("walk_ignore_and_filter_only", |b| {
        b.iter(|| {
            let walker = WalkerBuilder::new(grep.clone(), matcher.clone(), display.clone())
                .ignore_patterns(ignore_patterns.clone())
                .force_ignore_patterns(force_ignore_patterns.clone())
                .file_filters(file_filters.clone())
                .build();
            walker.walk(black_box(tree.path()));
        })
    });
}

fn search_bench(c: &mut Criterion) {
    let tree = bench_tree_sparse();
    let files = tree.collect_files();
    let matcher = build_matcher("needle");
    let display = build_display(tree.path());
    let grep = grep::grep();

    c.bench_function("search_sorted_sparse_file_list", |b| {
        b.iter(|| {
            run_grep_on_files(
                black_box(&files),
                matcher.clone(),
                display.clone(),
                grep.clone(),
            );
        })
    });
}

fn end_to_end_bench(c: &mut Criterion) {
    let tree = bench_tree_dense();
    let matcher = build_matcher("needle");
    let display = build_display(tree.path());
    let grep = grep::grep();
    let file_filters = Filters::new(&["*".to_string()]).unwrap();
    let ignore_patterns = Patterns::new(tree.path().to_str().unwrap(), &[]);
    let force_ignore_patterns = Patterns::new(tree.path().to_str().unwrap(), &[]);

    c.bench_function("walk_and_search_dense_tree", |b| {
        b.iter(|| {
            let walker = WalkerBuilder::new(grep.clone(), matcher.clone(), display.clone())
                .ignore_patterns(ignore_patterns.clone())
                .force_ignore_patterns(force_ignore_patterns.clone())
                .file_filters(file_filters.clone())
                .build();
            walker.walk(black_box(tree.path()));
        })
    });
}

criterion_group!(
    benches,
    patterns_bench,
    traversal_bench,
    search_bench,
    end_to_end_bench
);
criterion_main!(benches);
