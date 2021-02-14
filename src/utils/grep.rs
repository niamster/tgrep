use std::sync::Arc;

use log::{debug, error, warn};

use crate::utils::display::{Display, DisplayContext};
use crate::utils::lines::LinesReader;
use crate::utils::matcher::Matcher;

pub type Grep<T> = fn(reader: &T, matcher: Matcher, display: Arc<dyn Display>);

type OnMatch = Box<dyn Fn(DisplayContext) -> bool>;
type OnEnd = Box<dyn Fn(usize, usize)>;

fn generic_grep<T: LinesReader>(reader: &T, matcher: Matcher, on_match: OnMatch, on_end: OnEnd) {
    let mut matches = 0;
    let mut total = 0;
    match reader.lines() {
        Ok(lines) => {
            for (lno, line) in lines.enumerate() {
                match line {
                    Ok(line) => {
                        total += 1;
                        if let Some(needle) = matcher(&line) {
                            matches += 1;
                            if on_match(DisplayContext::new(lno, &line, needle)) {
                                break;
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
    on_end(total, matches);
}

pub fn grep<T: LinesReader>(reader: &T, matcher: Matcher, display: Arc<dyn Display>) {
    let path = reader.path().clone();
    let display = display.clone();
    generic_grep(
        reader,
        matcher,
        Box::new(move |context| {
            display.display(&path, Some(context));
            false
        }),
        Box::new(move |_, _| {}),
    );
}

pub fn grep_matches_once<T: LinesReader>(reader: &T, matcher: Matcher, display: Arc<dyn Display>) {
    let path = reader.path().clone();
    let display = display.clone();
    generic_grep(
        reader,
        matcher,
        Box::new(move |context| {
            display.display(&path, Some(context));
            true
        }),
        Box::new(move |_, _| {}),
    );
}

pub fn grep_matches_all_lines<T: LinesReader>(
    reader: &T,
    matcher: Matcher,
    display: Arc<dyn Display>,
) {
    let path = reader.path().clone();
    let display = display.clone();
    generic_grep(
        reader,
        matcher,
        Box::new(move |_| false),
        Box::new(move |total, matches| {
            if matches == total {
                display.display(&path, None);
            }
        }),
    );
}
