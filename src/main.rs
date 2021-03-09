use std::{
    path::{self, PathBuf},
    sync::Arc,
};

use anyhow::Error;
use futures::executor::ThreadPool;
use log::info;
use regex::RegexBuilder;
use structopt::StructOpt;

mod utils;

use crate::utils::display::{DisplayTerminal, Format, PathFormat};
use crate::utils::filters::Filters;
use crate::utils::grep;
use crate::utils::matcher::{Match, MatcherOptions};
use crate::utils::patterns::Patterns;
use crate::utils::stdin::Stdin;
use crate::utils::walker::WalkerBuilder;
use crate::utils::writer::StdoutWriter;

const GIT_DIR: &str = ".git/";

#[derive(Debug, StructOpt)]
struct Cli {
    #[structopt(short = "i", help = "Case-insensitive search")]
    ignore_case: bool,
    #[structopt(long = "ignore-symlinks", help = "Do not follow symlinks")]
    ignore_symlinks: bool,
    #[structopt(short = "v", help = "Invert the sense of matching")]
    invert: bool,
    #[structopt(short = "l", help = "Show only files with match")]
    path_only: bool,
    #[structopt(long = "no-colour", help = "Disable colours")]
    no_color: bool,
    #[structopt(
        short = "I",
        long = "ignore",
        default_value = GIT_DIR,
        number_of_values = 1,
        help = "Default ignore pattern"
    )]
    ignore_patterns: Vec<String>,
    #[structopt(
        short = "f",
        default_value = "*",
        help = "File filter pattern",
        number_of_values = 1,
        name = "filter-pattern"
    )]
    filter_pattern: Vec<String>,
    regexp: String,
    #[structopt(parse(from_os_str))]
    paths: Vec<PathBuf>,
    /// Pass many times for more log output
    ///
    /// By default, it'll only report errors. Passing `-V` one time also prints
    /// warnings, `-VV` enables info logging, `-VVV` debug, and `-VVVV` trace.
    #[structopt(long, short = "V", parse(from_occurrences))]
    verbosity: i8,
}

fn log_level(verbosity: i8) -> log::LevelFilter {
    match verbosity {
        std::i8::MIN..=-1 => log::LevelFilter::Off,
        0 => log::LevelFilter::Error,
        1 => log::LevelFilter::Warn,
        2 => log::LevelFilter::Info,
        3 => log::LevelFilter::Debug,
        4..=std::i8::MAX => log::LevelFilter::Trace,
    }
}

fn main() -> Result<(), Error> {
    let args = Cli::from_args();

    env_logger::Builder::new()
        .filter_level(log_level(args.verbosity))
        .parse_default_env()
        .init();

    let stdin = Stdin::new();
    let paths = if args.paths.is_empty() {
        if stdin.is_readable() {
            vec![]
        } else {
            vec![PathBuf::from(".")]
        }
    } else {
        args.paths
    };
    info!("regexp={:?}, paths={:?}", args.regexp, paths);

    let regexp = RegexBuilder::new(args.regexp.as_str())
        .case_insensitive(args.ignore_case)
        .build()?;
    let width = if let Some((width, _)) = term_size::dimensions() {
        width
    } else {
        usize::MAX
    };
    let tpool = ThreadPool::new()?;
    let file_filters = Filters::new(&args.filter_pattern)?;
    let matcher = {
        // Some fun stuff:
        // 1. https://github.com/rust-lang/rust/issues/22340
        // 2. https://github.com/rust-lang/rust/issues/26085
        // 3. https://github.com/rust-lang/rust/issues/29625
        let invert = args.invert;
        let regexp = regexp;
        move |line: &str, options| -> Option<Vec<Match>> {
            let option = if invert {
                Some(vec![Match::new(0, line.len())])
            } else {
                None
            };
            match options {
                MatcherOptions::FUZZY => {
                    let result = if let Some(pos) = regexp.shortest_match(line) {
                        Some(vec![Match::new(0, pos)])
                    } else {
                        None
                    };
                    result.xor(option)
                }
                MatcherOptions::EXACT(max) => {
                    let mut matches = vec![];
                    for (i, m) in regexp.find_iter(line).enumerate() {
                        matches.push(Match::new(m.start(), m.end()));
                        if i + 1 == max {
                            break;
                        }
                    }
                    if matches.is_empty() {
                        None
                    } else {
                        Some(matches)
                    }
                    .xor(option)
                }
            }
        }
    };
    let display = {
        let path_only = args.path_only;
        let no_color = args.no_color;
        move |path_format: PathFormat| {
            DisplayTerminal::new(
                width,
                if path_only {
                    Format::PathOnly { colour: !no_color }
                } else {
                    Format::Rich { colour: !no_color }
                },
                path_format,
                Arc::new(StdoutWriter::new()),
            )
        }
    };
    let ignore_patterns = {
        let mut ignore_patterns = vec![GIT_DIR.to_owned()];
        ignore_patterns.extend(args.ignore_patterns);
        ignore_patterns.dedup();
        ignore_patterns
    };
    for path in paths {
        let path = path.as_path();
        // See some fun at https://github.com/rust-lang/rfcs/issues/2208
        let prefix = path_clean::clean(path.to_str().unwrap()) + &path::MAIN_SEPARATOR.to_string();
        let fpath = path.canonicalize().unwrap();
        let path_format = {
            let fpath = fpath.clone();
            move |entry: &PathBuf| -> String {
                let entry = entry.as_path();
                let entry = entry.strip_prefix(&fpath).unwrap();
                prefix.clone() + entry.to_str().unwrap()
            }
        };
        let display = display(Arc::new(Box::new(path_format)));
        let ignore_patterns = Patterns::new(&fpath.as_path().to_str().unwrap(), &ignore_patterns);
        let grep = if args.path_only {
            if args.invert {
                grep::grep_matches_all_lines
            } else {
                grep::grep_matches_once
            }
        } else {
            grep::grep
        };
        let walker =
            WalkerBuilder::new(grep, Arc::new(Box::new(matcher.clone())), Arc::new(display))
                .thread_pool(tpool.clone())
                .ignore_patterns(ignore_patterns)
                .file_filters(file_filters.clone())
                .ignore_symlinks(args.ignore_symlinks)
                .build();
        walker.walk(&fpath);
    }
    if stdin.is_readable() {
        let path_format = |entry: &PathBuf| -> String { entry.to_str().unwrap().to_owned() };
        let display = display(Arc::new(Box::new(path_format)));
        grep::grep(
            Arc::new(stdin),
            Arc::new(Box::new(matcher)),
            Arc::new(display),
        );
    }

    Ok(())
}
