use std::{env, path::PathBuf};

use lazy_static::lazy_static;
use quicli::prelude::*;
use regex::Regex;
use structopt::StructOpt;

mod utils;

use crate::utils::display::DisplayTerminal;
use crate::utils::patterns::ToPatterns;
use crate::utils::walker::Walker;

const MARGIN: usize = 64;

lazy_static! {
    static ref DEFAULT_IGNORE_PATTERNS: Vec<String> = vec![".git".to_string()];
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
    let (_, ignore_patterns) = if args.ignore_patterns.is_empty() {
        DEFAULT_IGNORE_PATTERNS.to_patterns()
    } else {
        args.ignore_patterns.to_patterns()
    };
    let ignore_files = if args.ignore_files.is_empty() {
        DEFAULT_IGNORE_FILES.clone()
    } else {
        args.ignore_files
    };
    let regexp = if args.ignore_case {
        "(?i)".to_string() + &args.regexp
    } else {
        args.regexp.clone()
    };
    info!("regexp={:?}, paths={:?}", args.regexp, paths);

    let regexp = Regex::new(regexp.as_str())?;
    let display = DisplayTerminal::new(MARGIN);
    let walker = Walker::new(ignore_patterns, &ignore_files, &regexp, &display);
    for path in paths {
        walker.walk(&path);
    }

    Ok(())
}
