use std::cmp;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use log::error;

use crate::utils::display::{Display, DisplayContext};
use crate::utils::lines::LinesReader;
use crate::utils::matcher::{Matcher, MatcherOptions};

pub type Grep = Arc<Box<dyn Fn(Arc<dyn LinesReader>, Matcher, Arc<dyn Display>) + Send + Sync>>;

type OnMatch = Box<dyn Fn(DisplayContext) -> bool>;
type OnEnd = Box<dyn Fn(usize, usize)>;

fn fuzzy_grep(reader: &Arc<dyn LinesReader>, matcher: &Matcher) -> Option<()> {
    reader
        .map()
        .ok()
        .and_then(|map| matcher(map, MatcherOptions::Fuzzy).and(Some(())))
}

fn generic_grep(reader: Arc<dyn LinesReader>, matcher: Matcher, on_match: OnMatch, on_end: OnEnd) {
    if fuzzy_grep(&reader, &matcher).is_none() {
        on_end(0, 0);
        return;
    }
    let mut matches = 0;
    let mut total = 0;
    match reader.lines() {
        Ok(mut lines) => {
            while let Some(line) = lines.next() {
                total += 1;
                if let Some(needle) = matcher(line, MatcherOptions::Exact(usize::MAX)) {
                    matches += 1;
                    if on_match(DisplayContext::new(total, line.to_string(), needle)) {
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
    if fuzzy_grep(&reader, &matcher).is_none() {
        return;
    }
    let path = reader.path().clone();
    let mut lqueue: VecDeque<String> = VecDeque::with_capacity(before + 1);
    let mut lno = 0;
    let mut pcount: isize = 0;
    let mut output = BTreeMap::new();
    match reader.lines() {
        Ok(mut lines) => {
            while let Some(line) = lines.next() {
                lno += 1;
                let needle = matcher(line, MatcherOptions::Exact(usize::MAX));

                if pcount > 0 {
                    output.entry(lno).or_insert_with(|| {
                        DisplayContext::with_lno_separator(lno, line.to_owned(), vec![], "-")
                    });
                    pcount -= 1;
                }
                if let Some(needle) = needle {
                    for i in 0..cmp::min(before, lqueue.len()) {
                        output.entry(lno - i - 1).or_insert_with(|| {
                            DisplayContext::with_lno_separator(
                                lno - i - 1,
                                lqueue.pop_front().unwrap(),
                                vec![],
                                "-",
                            )
                        });
                    }
                    output.insert(lno, DisplayContext::new(lno, line.to_owned(), needle));
                    pcount = after as isize;
                }
                lqueue.push_back(line.to_string());
                if lqueue.len() == before + 1 {
                    lqueue.pop_front();
                }
            }
            let mut plno = 0;
            for (lno, context) in output {
                if plno > 0 && lno - plno > 1 {
                    display.match_separator();
                }
                plno = lno;
                display.display(&path, Some(context));
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

pub fn grep_count() -> Grep {
    Arc::new(Box::new(
        move |reader: Arc<dyn LinesReader>, matcher: Matcher, display: Arc<dyn Display>| {
            let path = reader.path().clone();
            let display = display.clone();
            generic_grep(
                reader,
                matcher,
                Box::new(move |_| false),
                Box::new(move |_, matches| {
                    if matches > 0 {
                        display.display(
                            &path,
                            Some(DisplayContext::new(matches, "".to_string(), vec![])),
                        );
                    }
                }),
            );
        },
    ))
}
