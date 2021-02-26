use std::sync::Arc;

use log::error;

use crate::utils::display::{Display, DisplayContext};
use crate::utils::lines::LinesReader;
use crate::utils::matcher::{Matcher, MatcherOptions};

pub type Grep = fn(reader: Arc<dyn LinesReader>, matcher: Matcher, display: Arc<dyn Display>);

type OnMatch = Box<dyn Fn(DisplayContext) -> bool>;
type OnEnd = Box<dyn Fn(usize, usize)>;

fn generic_grep(reader: Arc<dyn LinesReader>, matcher: Matcher, on_match: OnMatch, on_end: OnEnd) {
    if let Ok(map) = reader.map() {
        if matcher(&map, MatcherOptions::FUZZY).is_none() {
            return;
        }
    }
    let mut matches = 0;
    let mut total = 0;
    match reader.lines() {
        Ok(mut lines) => {
            while let Some(line) = lines.next() {
                total += 1;
                if let Some(needle) = matcher(&line, MatcherOptions::EXACT(usize::MAX)) {
                    matches += 1;
                    if on_match(DisplayContext::new(total, &line, needle)) {
                        break;
                    }
                }
            }
        }
        Err(e) => error!("Failed to read '{}': {}", reader.path().display(), e),
    }
    on_end(total, matches);
}

pub fn grep(reader: Arc<dyn LinesReader>, matcher: Matcher, display: Arc<dyn Display>) {
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

pub fn grep_matches_once(
    reader: Arc<dyn LinesReader>,
    matcher: Matcher,
    display: Arc<dyn Display>,
) {
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

pub fn grep_matches_all_lines(
    reader: Arc<dyn LinesReader>,
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
