use std::{env, path::PathBuf, sync::Arc};

use futures::executor::ThreadPool;
use quicli::prelude::*;
use regex::RegexBuilder;
use structopt::StructOpt;

mod utils;

use crate::utils::display::DisplayTerminal;
use crate::utils::filters::Filters;
use crate::utils::patterns::Patterns;
use crate::utils::walker::Walker;

#[derive(Debug, StructOpt)]
struct Cli {
    #[structopt(short = "i", help = "Case-insensitive search")]
    ignore_case: bool,
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
        name = "filter-pattern"
    )]
    filter_pattern: String,
    regexp: String,
    #[structopt(parse(from_os_str))]
    paths: Vec<PathBuf>,
    #[structopt(flatten)]
    verbosity: Verbosity,
}

fn main() -> CliResult {
    let args = Cli::from_args();
    args.verbosity.setup_env_logger("tgrep")?;

    let paths = if args.paths.is_empty() {
        vec![env::current_dir()?]
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
    let display = DisplayTerminal::new(width);
    let tpool = ThreadPool::new()?;
    let file_filters = Filters::new(&[args.filter_pattern])?;
    for path in paths {
        let path = path.as_path().canonicalize().unwrap();
        let ignore_patterns =
            Patterns::new(&path.as_path().to_str().unwrap(), &args.ignore_patterns);
        let walker = Walker::new(
            tpool.clone(),
            ignore_patterns,
            args.ignore_files.clone(),
            file_filters.clone(),
            regexp.clone(),
            Arc::new(display.clone()),
        );
        walker.walk(&path);
    }

    Ok(())
}
