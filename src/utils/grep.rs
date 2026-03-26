use std::cmp;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use log::error;

use crate::utils::display::{Display, DisplayContext};
use crate::utils::lines::LinesReader;
use crate::utils::matcher::{Match, Matcher, MatcherOptions};
use crate::utils::timing;

pub type Grep = Arc<Box<dyn Fn(Arc<dyn LinesReader>, Matcher, Arc<dyn Display>) + Send + Sync>>;

type OnMatch = Box<dyn Fn(DisplayContext) -> bool>;
type OnEnd = Box<dyn Fn(usize, usize)>;

fn fuzzy_grep(reader: &Arc<dyn LinesReader>, matcher: &Matcher) -> Option<()> {
    let res = timing::time("reader.map", || reader.map());
    if res.is_err() {
        // Some readers do not support map
        return Some(());
    }
    res.ok().and_then(|map| {
        timing::time("grep.fuzzy", || {
            matcher(map, MatcherOptions::Fuzzy).and(Some(()))
        })
    })
}

fn generic_grep(reader: Arc<dyn LinesReader>, matcher: Matcher, on_match: OnMatch, on_end: OnEnd) {
    if fuzzy_grep(&reader, &matcher).is_none() {
        on_end(0, 0);
        return;
    }
    let mut matches = 0;
    let mut total = 0;
    match timing::time("reader.lines", || reader.lines()) {
        Ok(mut lines) => {
            while let Some(line) = lines.next() {
                total += 1;
                if let Some(needle) = timing::time("grep.exact", || {
                    matcher(line, MatcherOptions::Exact(usize::MAX))
                }) {
                    matches += 1;
                    if timing::time("display.match", || {
                        on_match(DisplayContext::new(total, line.to_string(), needle))
                    }) {
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
                    timing::time("display.separator", || display.match_separator());
                }
                plno = lno;
                timing::time("display.match", || display.display(&path, Some(context)));
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
                    if matches == total && total != 0 {
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
                        let matches = matches.to_string();
                        let matches_len = matches.len();
                        display.display(
                            &path,
                            Some(DisplayContext::new(
                                0,
                                matches,
                                vec![Match::new(0, matches_len)],
                            )),
                        );
                    }
                }),
            );
        },
    ))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use streaming_iterator::StreamingIterator;

    use super::*;
    use crate::utils::display::{DisplayTerminal, Format, PathFormat};
    use crate::utils::lines::LineIterator;
    use crate::utils::writer::Writer;

    #[derive(Clone)]
    struct TestWriter {
        writes: Arc<Mutex<Vec<String>>>,
    }

    impl TestWriter {
        fn new() -> Self {
            Self {
                writes: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn lines(&self) -> Vec<String> {
            self.writes.lock().unwrap().clone()
        }
    }

    impl Writer for TestWriter {
        fn write(&self, content: &str) {
            self.writes.lock().unwrap().push(content.to_owned());
        }
    }

    struct TestReader {
        path: PathBuf,
        content: String,
        lines: Vec<String>,
        map_supported: bool,
        lines_called: Arc<AtomicUsize>,
    }

    impl TestReader {
        fn new(path: &str, lines: &[&str], map_supported: bool) -> Self {
            Self {
                path: PathBuf::from(path),
                content: lines.join("\n"),
                lines: lines.iter().map(|line| (*line).to_owned()).collect(),
                map_supported,
                lines_called: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn lines_called(&self) -> usize {
            self.lines_called.load(Ordering::Relaxed)
        }
    }

    impl LinesReader for TestReader {
        fn map(&self) -> anyhow::Result<&str> {
            if self.map_supported {
                Ok(&self.content)
            } else {
                anyhow::bail!("not supported")
            }
        }

        fn lines(&self) -> anyhow::Result<Box<LineIterator>> {
            self.lines_called.fetch_add(1, Ordering::Relaxed);
            Ok(Box::new(TestLineIterator::new(self.lines.clone())))
        }

        fn path(&self) -> &PathBuf {
            &self.path
        }
    }

    struct TestLineIterator {
        lines: Vec<String>,
        pos: usize,
    }

    impl TestLineIterator {
        fn new(lines: Vec<String>) -> Self {
            Self { lines, pos: 0 }
        }
    }

    impl StreamingIterator for TestLineIterator {
        type Item = str;

        fn advance(&mut self) {
            if self.pos < self.lines.len() {
                self.pos += 1;
            }
        }

        fn get(&self) -> Option<&Self::Item> {
            self.lines.get(self.pos).map(String::as_str)
        }

        fn next(&mut self) -> Option<&Self::Item> {
            let line = self.lines.get(self.pos).map(String::as_str);
            if line.is_some() {
                self.pos += 1;
            }
            line
        }
    }

    fn matcher(needle: &'static str) -> Matcher {
        Arc::new(Box::new(move |line: &str, options| match options {
            MatcherOptions::Fuzzy => line.find(needle).map(|_| vec![Match::new(0, needle.len())]),
            MatcherOptions::Exact(max) => {
                let mut matches = Vec::new();
                let mut start = 0;
                while let Some(offset) = line[start..].find(needle) {
                    let start_offset = start + offset;
                    matches.push(Match::new(start_offset, start_offset + needle.len()));
                    start = start_offset + needle.len();
                    if matches.len() == max {
                        break;
                    }
                }
                if matches.is_empty() {
                    None
                } else {
                    Some(matches)
                }
            }
        }))
    }

    fn display(format: Format, writer: Arc<dyn Writer>) -> Arc<dyn Display> {
        let path_format: PathFormat = Arc::new(Box::new(|path: &Path| {
            path.file_name().unwrap().to_string_lossy().into_owned()
        }));
        Arc::new(DisplayTerminal::new(120, format, path_format, writer))
    }

    #[test]
    fn grep_outputs_all_matching_lines() {
        let writer = TestWriter::new();
        let reader = Arc::new(TestReader::new(
            "sample.txt",
            &["alpha", "beta needle", "needle gamma"],
            true,
        ));

        grep()(
            reader,
            matcher("needle"),
            display(
                Format::Rich {
                    colour: false,
                    match_only: false,
                    no_path: false,
                    no_lno: false,
                },
                Arc::new(writer.clone()),
            ),
        );

        assert_eq!(
            vec!["sample.txt:2: beta needle", "sample.txt:3: needle gamma"],
            writer.lines()
        );
    }

    #[test]
    fn grep_skips_line_iteration_when_fuzzy_match_fails() {
        let writer = TestWriter::new();
        let reader = Arc::new(TestReader::new("sample.txt", &["alpha", "beta"], true));

        grep()(
            reader.clone(),
            matcher("needle"),
            display(
                Format::Rich {
                    colour: false,
                    match_only: false,
                    no_path: false,
                    no_lno: false,
                },
                Arc::new(writer.clone()),
            ),
        );

        assert_eq!(0, reader.lines_called());
        assert!(writer.lines().is_empty());
    }

    #[test]
    fn grep_with_context_prints_context_and_separator_between_groups() {
        let writer = TestWriter::new();
        let reader = Arc::new(TestReader::new(
            "sample.txt",
            &[
                "alpha",
                "needle one",
                "gamma",
                "delta",
                "epsilon",
                "needle two",
                "zeta",
            ],
            false,
        ));

        grep_with_context(1, 1)(
            reader,
            matcher("needle"),
            display(
                Format::Rich {
                    colour: false,
                    match_only: false,
                    no_path: false,
                    no_lno: false,
                },
                Arc::new(writer.clone()),
            ),
        );

        assert_eq!(
            vec![
                "sample.txt-1- alpha",
                "sample.txt:2: needle one",
                "sample.txt-3- gamma",
                "..",
                "sample.txt-5- epsilon",
                "sample.txt:6: needle two",
                "sample.txt-7- zeta",
            ],
            writer.lines()
        );
    }

    #[test]
    fn grep_matches_once_stops_after_first_match() {
        let writer = TestWriter::new();
        let reader = Arc::new(TestReader::new(
            "sample.txt",
            &["needle one", "needle two"],
            false,
        ));

        grep_matches_once()(
            reader,
            matcher("needle"),
            display(
                Format::Rich {
                    colour: false,
                    match_only: false,
                    no_path: false,
                    no_lno: false,
                },
                Arc::new(writer.clone()),
            ),
        );

        assert_eq!(vec!["sample.txt:1: needle one"], writer.lines());
    }

    #[test]
    fn grep_matches_all_lines_prints_only_matching_path() {
        let writer = TestWriter::new();
        let reader = Arc::new(TestReader::new(
            "sample.txt",
            &["needle one", "needle two"],
            false,
        ));

        grep_matches_all_lines()(
            reader,
            matcher("needle"),
            display(Format::PathOnly { colour: false }, Arc::new(writer.clone())),
        );

        assert_eq!(vec!["sample.txt"], writer.lines());
    }

    #[test]
    fn grep_count_prints_number_of_matching_lines() {
        let writer = TestWriter::new();
        let reader = Arc::new(TestReader::new(
            "sample.txt",
            &["needle one", "other", "needle two"],
            false,
        ));

        grep_count()(
            reader,
            matcher("needle"),
            display(
                Format::Rich {
                    colour: false,
                    match_only: false,
                    no_path: false,
                    no_lno: false,
                },
                Arc::new(writer.clone()),
            ),
        );

        assert_eq!(vec!["sample.txt:0: 2"], writer.lines());
    }
}
