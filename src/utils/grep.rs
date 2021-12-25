use std::cmp;
use std::collections::VecDeque;
use std::sync::Arc;

use log::error;

use crate::utils::display::{Display, DisplayContext};
use crate::utils::lines::LinesReader;
use crate::utils::matcher::{Matcher, MatcherOptions};

pub type Grep = Arc<Box<dyn Fn(Arc<dyn LinesReader>, Matcher, Arc<dyn Display>) + Send + Sync>>;

type OnMatch = Box<dyn Fn(DisplayContext) -> bool>;
type OnEnd = Box<dyn Fn(usize, usize)>;

fn generic_grep(reader: Arc<dyn LinesReader>, matcher: Matcher, on_match: OnMatch, on_end: OnEnd) {
    if let Ok(map) = reader.map() {
        if matcher(map, MatcherOptions::Fuzzy).is_none() {
            on_end(0, 0);
            return;
        }
    }
    let mut matches = 0;
    let mut total = 0;
    match reader.lines() {
        Ok(mut lines) => {
            while let Some(line) = lines.next() {
                total += 1;
                if let Some(needle) = matcher(line, MatcherOptions::Exact(usize::MAX)) {
                    matches += 1;
                    if on_match(DisplayContext::new(total, line, needle)) {
                        break;
                    }
                }
            }
        }
        Err(e) => error!("Failed to read '{}': {}", reader.path().display(), e),
    }
    on_end(total, matches);
}

pub fn grep() -> Grep {
    Arc::new(Box::new(
        move |reader: Arc<dyn LinesReader>, matcher: Matcher, display: Arc<dyn Display>| {
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
        },
    ))
}

fn _grep_with_context(
    reader: Arc<dyn LinesReader>,
    matcher: Matcher,
    display: Arc<dyn Display>,
    before: usize,
    after: usize,
) {
    if let Ok(map) = reader.map() {
        if matcher(map, MatcherOptions::Fuzzy).is_none() {
            return;
        }
    }
    let path = reader.path().clone();
    let mut lqueue: VecDeque<String> = VecDeque::with_capacity(before + after + 1);
    let mut lno = 0;
    let mut pcount: isize = 0;
    match reader.lines() {
        Ok(mut lines) => {
            while let Some(line) = lines.next() {
                lno += 1;
                if pcount > 0 {
                    display.display(
                        &path,
                        Some(DisplayContext::with_lno_separator(lno, line, vec![], "+")),
                    );
                    pcount -= 1;
                }
                if let Some(needle) = matcher(line, MatcherOptions::Exact(usize::MAX)) {
                    for i in 0..cmp::min(before, lqueue.len()) {
                        display.display(
                            &path,
                            Some(DisplayContext::with_lno_separator(
                                lno - i - 1,
                                lqueue.get(i).unwrap(),
                                vec![],
                                "-",
                            )),
                        );
                    }
                    display.display(&path, Some(DisplayContext::new(lno, line, needle)));
                    pcount = after as isize;
                }
                lqueue.push_back(line.to_string());
                if lqueue.len() == before + 1 {
                    lqueue.pop_front();
                }
            }
        }
        Err(e) => error!("Failed to read '{}': {}", reader.path().display(), e),
    }
}

pub fn grep_with_context(before: usize, after: usize) -> Grep {
    Arc::new(Box::new(
        move |reader: Arc<dyn LinesReader>, matcher: Matcher, display: Arc<dyn Display>| {
            _grep_with_context(reader, matcher, display, before, after)
        },
    ))
}

pub fn grep_matches_once() -> Grep {
    Arc::new(Box::new(
        move |reader: Arc<dyn LinesReader>, matcher: Matcher, display: Arc<dyn Display>| {
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
        },
    ))
}

pub fn grep_matches_all_lines() -> Grep {
    Arc::new(Box::new(
        move |reader: Arc<dyn LinesReader>, matcher: Matcher, display: Arc<dyn Display>| {
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
        },
    ))
}
