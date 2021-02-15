use std::{
    cmp,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use ansi_term::Colour;

use crate::utils::matcher::Match;

type Range = std::ops::Range<usize>;

pub struct DisplayContext<'a> {
    lno: usize,
    line: &'a str,
    needle: Vec<Match>,
}

impl<'a> DisplayContext<'a> {
    pub fn new(lno: usize, line: &'a str, needle: Vec<Match>) -> Self {
        DisplayContext { lno, line, needle }
    }
}

pub trait Display: Send + Sync {
    fn display(&self, path: &PathBuf, context: Option<DisplayContext>);
}

pub type PathFormat = Arc<Box<dyn Fn(&PathBuf) -> String + Send + Sync>>;

pub trait OutputFormat: Send + Sync {
    fn format(&self, width: usize, path: &str, context: Option<DisplayContext>) -> String;
}

#[derive(Clone)]
pub struct DisplayTerminal<T> {
    lock: Arc<Mutex<()>>,
    width: usize,
    format: T,
    path_format: PathFormat,
}

impl<T> DisplayTerminal<T>
where
    T: OutputFormat,
{
    pub fn new(width: usize, format: T, path_format: PathFormat) -> Self {
        DisplayTerminal {
            lock: Arc::new(Mutex::new(())),
            width,
            format,
            path_format,
        }
    }
}

impl<T> Display for DisplayTerminal<T>
where
    T: OutputFormat,
{
    fn display(&self, path: &PathBuf, context: Option<DisplayContext>) {
        let formated = self
            .format
            .format(self.width, &(self.path_format)(path), context);
        let guard = self.lock.lock();
        println!("{}", formated);
        drop(guard);
    }
}

pub enum Format {
    Rich { colour: bool },
    PathOnly { colour: bool },
}

impl Format {
    fn rich_format_many(
        &self,
        _width: usize,
        path: &str,
        lno: usize,
        line: &str,
        needles: Vec<Range>,
        colour: bool,
    ) -> String {
        assert!(needles.len() >= 2);
        let lno = lno.to_string();
        let mut formatted = Vec::with_capacity(2 * needles.len() + 2);
        formatted.push(if colour {
            format!("{}:{} ", Colour::Blue.paint(path), Colour::Green.paint(lno))
        } else {
            format!("{}:{} ", path, lno)
        });
        for (idx, needle) in needles.iter().enumerate() {
            assert!(needle.end >= needle.start);
            assert!(needle.end <= line.len());
            if idx == 0 {
                if needle.start > 0 {
                    formatted.push(line[..needle.start].to_string());
                }
            } else {
                let prev = &needles[idx - 1];
                formatted.push(line[prev.end..needle.start].to_string());
            }
            let what = &line[needle.start..needle.end];
            formatted.push(if colour {
                Colour::Red.paint(what).to_string()
            } else {
                what.to_string()
            });
        }
        let last = needles.last().unwrap();
        if last.end < line.len() {
            formatted.push(line[last.end..].to_string());
        }
        formatted.join("")
    }

    fn rich_format_one(
        &self,
        width: usize,
        path: &str,
        lno: usize,
        line: &str,
        needle: &Range,
        colour: bool,
    ) -> String {
        assert!(needle.end >= needle.start);
        assert!(needle.end <= line.len());
        let lno = lno.to_string();
        let needle_len = needle.end - needle.start;
        let preambule_len = path.len() + lno.len() + 2; // +2 for `: ` in format
        let width = cmp::max(width, preambule_len + needle_len);
        let width = width - preambule_len;
        let width = if width < needle_len {
            needle_len
        } else {
            width
        };
        let (left_margin, right_margin) = if width == needle_len {
            (usize::MAX, usize::MAX)
        } else if needle.start < width / 2 {
            let left_margin = cmp::min(needle.start, (width - needle_len) / 2);
            let right_margin = width - needle_len - left_margin;
            (left_margin, right_margin)
        } else {
            let right_margin = cmp::min(line.len() - needle.end, (width - needle_len) / 2);
            let left_margin = width - needle_len - right_margin;
            (left_margin, right_margin)
        };
        let (start, prefix) = if left_margin == usize::MAX {
            (needle.start, "")
        } else if needle.start > left_margin {
            let prefix = "[...] ";
            (needle.start - left_margin + prefix.len(), prefix)
        } else {
            (0, "")
        };
        let (end, suffix) = if right_margin == usize::MAX {
            (needle.end, "")
        } else if line.len() - needle.end > right_margin {
            let suffix = " [...]";
            (needle.end + right_margin - suffix.len(), suffix)
        } else {
            (line.len(), "")
        };
        let before = &line[start..needle.start];
        let what = &line[needle.start..needle.end];
        let after = &line[needle.end..end];
        if colour {
            format!(
                "{}:{} {}{}{}{}{}",
                Colour::Blue.paint(path),
                Colour::Green.paint(lno),
                Colour::Purple.paint(prefix),
                before,
                Colour::Red.paint(what),
                after,
                Colour::Purple.paint(suffix),
            )
        } else {
            format!(
                "{}:{} {}{}{}{}{}",
                path, lno, prefix, before, what, after, suffix,
            )
        }
    }

    fn rich_format(
        &self,
        width: usize,
        path: &str,
        lno: usize,
        line: &str,
        needles: Vec<Range>,
        colour: bool,
    ) -> String {
        if needles.len() == 1 {
            self.rich_format_one(width, path, lno, line, &needles[0], colour)
        } else {
            self.rich_format_many(width, path, lno, line, needles, colour)
        }
    }

    fn format_path(&self, path: &str, colour: bool) -> String {
        if colour {
            Colour::Blue.paint(path).to_string()
        } else {
            path.to_string()
        }
    }
}

impl OutputFormat for Format {
    fn format(&self, width: usize, path: &str, context: Option<DisplayContext>) -> String {
        match self {
            Format::Rich { colour } => match context {
                Some(ctx) => self.rich_format(
                    width,
                    path,
                    ctx.lno,
                    ctx.line,
                    ctx.needle.into_iter().map(Into::into).collect(),
                    *colour,
                ),
                None => self.format_path(path, *colour),
            },
            Format::PathOnly { colour } => self.format_path(path, *colour),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_format() {
        let test = |width, len, needle: Range, start, end, prefix, suffix| {
            let prefix = if prefix { "[...] " } else { "" };
            let suffix = if suffix { " [...]" } else { "" };
            let needle_len = needle.end - needle.start;
            let preambule = "/:0 ";
            let formated = format!(
                "{}{}{}{}{}{}",
                preambule,
                prefix,
                "-".repeat(start),
                "-".repeat(needle_len),
                "-".repeat(end),
                suffix,
            );
            assert_eq!(
                formated,
                Format::Rich { colour: false }.format(
                    width,
                    "/",
                    Some(DisplayContext::new(
                        0,
                        &"-".repeat(len),
                        vec![needle.into()]
                    ))
                ),
            );
            assert_eq!(
                if len < width - preambule.len() {
                    len + preambule.len()
                } else if needle_len > width {
                    needle_len + preambule.len()
                } else {
                    width
                },
                formated.len()
            );
        };
        test(40, 80, Range { start: 4, end: 5 }, 4, 25, false, true);
        test(40, 80, Range { start: 64, end: 65 }, 14, 15, true, false);
        test(40, 80, Range { start: 34, end: 45 }, 7, 6, true, true);
        test(40, 80, Range { start: 4, end: 45 }, 0, 0, false, false);
        test(40, 80, Range { start: 4, end: 75 }, 0, 0, false, false);
        test(120, 80, Range { start: 4, end: 75 }, 4, 5, false, false);
        test(40, 80, Range { start: 0, end: 80 }, 0, 0, false, false);
        test(120, 80, Range { start: 0, end: 80 }, 0, 0, false, false);
        test(40, 80, Range { start: 10, end: 80 }, 0, 0, false, false);
        test(120, 80, Range { start: 10, end: 80 }, 10, 0, false, false);
        test(120, 80, Range { start: 0, end: 70 }, 0, 10, false, false);
    }
}
