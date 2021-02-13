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

use crate::utils::display::{DisplayTerminal, Format};
use crate::utils::filters::Filters;
use crate::utils::matcher::Match;
use crate::utils::patterns::Patterns;
use crate::utils::walker::Walker;

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
        long = "ignore",
        default_value = ".git/",
        help = "Default ignore pattern"
    )]
    ignore_patterns: Vec<String>,
    #[structopt(
        long = "ignore-file",
        default_value = ".gitignore",
        help = "Default ignore file name"
    )]
    ignore_files: Vec<String>,
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

    let paths = if args.paths.is_empty() {
        vec![PathBuf::from(".")]
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
        // Some fun stuff:
        // 1. https://github.com/rust-lang/rust/issues/22340
        // 2. https://github.com/rust-lang/rust/issues/26085
        // 3. https://github.com/rust-lang/rust/issues/29625
        let matcher = {
            let regexp = regexp.clone();
            let invert = args.invert;
            move |line: &str| -> Option<Match> {
                if line.is_empty() {
                    return None;
                }
                let option = if invert {
                    Some(Match::new(0, line.len()))
                } else {
                    None
                };
                regexp
                    .find(line)
                    .map(|v| Match::new(v.start(), v.end()))
                    .xor(option)
            }
        };
        let display = DisplayTerminal::new(
            width,
            if args.path_only {
                Format::PathOnly
            } else {
                Format::Rich {
                    colour: !args.no_color,
                }
            },
            Arc::new(Box::new(path_format)),
        );
        let ignore_patterns =
            Patterns::new(&fpath.as_path().to_str().unwrap(), &args.ignore_patterns);
        let walker = Walker::new(
            tpool.clone(),
            ignore_patterns,
            args.ignore_files.clone(),
            file_filters.clone(),
            Arc::new(Box::new(matcher)),
            if args.path_only { 1 } else { usize::MAX },
            args.ignore_symlinks,
            Arc::new(display),
        );
        walker.walk(&fpath);
    }

    Ok(())
}
