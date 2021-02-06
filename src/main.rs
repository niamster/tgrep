use std::{env, path::PathBuf, sync::Arc};

use lazy_static::lazy_static;
use quicli::prelude::*;
use regex::RegexBuilder;
use structopt::StructOpt;

mod utils;

use crate::utils::display::DisplayTerminal;
use crate::utils::patterns::Patterns;
use crate::utils::walker::Walker;

const MARGIN: usize = 64;

lazy_static! {
    static ref DEFAULT_IGNORE_PATTERNS: Vec<String> = vec![".git/".to_string()];
    static ref DEFAULT_IGNORE_FILES: Vec<String> = vec![".gitignore".to_string()];
}

#[derive(Debug, StructOpt)]
struct Cli {
    #[structopt(short = "i")]
    ignore_case: bool,
    #[structopt(long = "ignore")]
    ignore_patterns: Vec<String>,
    #[structopt(long = "ignore-file")]
    ignore_files: Vec<String>,
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
    let ignore_patterns = if args.ignore_patterns.is_empty() {
        DEFAULT_IGNORE_PATTERNS.clone()
    } else {
        args.ignore_patterns
    };
    let ignore_files = if args.ignore_files.is_empty() {
        DEFAULT_IGNORE_FILES.clone()
    } else {
        args.ignore_files
    };
    info!("regexp={:?}, paths={:?}", args.regexp, paths);

    let regexp = RegexBuilder::new(args.regexp.as_str())
        .case_insensitive(args.ignore_case)
        .build()?;
    let display = DisplayTerminal::new(MARGIN);
    for path in paths {
        let path = path.as_path().canonicalize().unwrap();
        let ignore_patterns = Patterns::new(&path.as_path().to_str().unwrap(), &ignore_patterns);
        let walker = Walker::new(
            ignore_patterns,
            Box::new(ignore_files.clone()),
            Box::new(regexp.clone()),
            Arc::new(Box::new(display.clone())),
        );
        walker.walk(&path);
    }

    Ok(())
}
