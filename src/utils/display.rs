use std::{cmp, path::Path, sync::Arc};

use ansi_term::Colour;

use crate::utils::matcher::Match;
use crate::utils::writer::Writer;

type Range = std::ops::Range<usize>;

pub struct DisplayContext<'a> {
    lno: usize,
    line: String,
    needle: Vec<Match>,
    lno_sep: &'a str,
}

impl<'a> DisplayContext<'a> {
    pub fn new(lno: usize, line: String, needle: Vec<Match>) -> Self {
        DisplayContext {
            lno,
            line,
            needle,
            lno_sep: ":",
        }
    }

    pub fn with_lno_separator(
        lno: usize,
        line: String,
        needle: Vec<Match>,
        lno_sep: &'a str,
    ) -> Self {
        let mut ctx = Self::new(lno, line, needle);
        ctx.lno_sep = lno_sep;
        ctx
    }
}

pub trait Display: Send + Sync {
    fn display(&self, path: &Path, context: Option<DisplayContext>);
    fn file_separator(&self);
    fn match_separator(&self);
    fn writer(&self) -> Arc<dyn Writer>;
    fn with_writer(&self, writer: Arc<dyn Writer>) -> Arc<dyn Display>;
}

pub type PathFormat = Arc<Box<dyn Fn(&Path) -> String + Send + Sync>>;

pub trait OutputFormat: Send + Sync {
    fn format(&self, width: usize, path: &str, context: Option<DisplayContext>) -> String;
    fn file_separator(&self) -> String;
    fn match_separator(&self) -> String;
}

#[derive(Clone)]
pub struct DisplayTerminal<T>
where
    T: Clone,
{
    width: usize,
    format: T,
    path_format: PathFormat,
    writer: Arc<dyn Writer>,
}

impl<T> DisplayTerminal<T>
where
    T: OutputFormat + Clone,
{
    pub fn new(width: usize, format: T, path_format: PathFormat, writer: Arc<dyn Writer>) -> Self {
        DisplayTerminal {
            width,
            format,
            path_format,
            writer,
        }
    }
}

impl<T> Display for DisplayTerminal<T>
where
    T: OutputFormat + Clone + 'static,
{
    fn display(&self, path: &Path, context: Option<DisplayContext>) {
        let formated = self
            .format
            .format(self.width, &(self.path_format)(path), context);
        self.writer.write(&formated);
    }

    fn file_separator(&self) {
        let separator = self.format.file_separator();
        self.writer.write(&separator);
    }

    fn match_separator(&self) {
        let separator = self.format.match_separator();
        self.writer.write(&separator);
    }

    fn writer(&self) -> Arc<dyn Writer> {
        self.writer.clone()
    }

    fn with_writer(&self, writer: Arc<dyn Writer>) -> Arc<dyn Display> {
        Arc::new(DisplayTerminal::new(
            self.width,
            self.format.clone(),
            self.path_format.clone(),
            writer,
        ))
    }
}

#[derive(Clone)]
pub enum Format {
    Rich {
        colour: bool,
        match_only: bool,
        no_path: bool,
        no_lno: bool,
    },
    PathOnly {
        colour: bool,
    },
}

impl Format {
    fn rich_format_many(
        &self,
        _width: usize,
        line: &str,
        needles: Vec<Range>,
        colour: bool,
    ) -> String {
        assert!(needles.len() >= 2);
        let mut formatted = Vec::with_capacity(2 * needles.len() + 2);
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

    fn rich_format_one(&self, width: usize, line: &str, needle: &Range, colour: bool) -> String {
        assert!(needle.end >= needle.start);
        assert!(needle.end <= line.len());
        let needle_len = needle.end - needle.start;
        let width = cmp::max(width, needle_len);
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
            let mut offset = needle.start - left_margin + prefix.len();
            if offset >= needle.start {
                (0, "")
            } else {
                while line.get(offset..needle.start) == None {
                    offset += 1;
                }
                (offset, prefix)
            }
        } else {
            (0, "")
        };
        let (end, suffix) = if right_margin == usize::MAX {
            (needle.end, "")
        } else if line.len() - needle.end > right_margin {
            let suffix = " [...]";
            let mut offset = needle.end + right_margin - suffix.len();
            if needle.end >= offset {
                (line.len(), "")
            } else {
                while line.get(needle.end..offset) == None {
                    offset -= 1;
                }
                (offset, suffix)
            }
        } else {
            (line.len(), "")
        };
        let before = &line[start..needle.start];
        let what = &line[needle.start..needle.end];
        let after = &line[needle.end..end];
        if colour {
            format!(
                "{}{}{}{}{}",
                Colour::Purple.paint(prefix),
                before,
                Colour::Red.paint(what),
                after,
                Colour::Purple.paint(suffix),
            )
        } else {
            format!("{}{}{}{}{}", prefix, before, what, after, suffix)
        }
    }

    fn rich_format_needles_only(
        &self,
        prefix: &str,
        line: &str,
        needles: Vec<Range>,
        colour: bool,
    ) -> String {
        let mut output = Vec::with_capacity(needles.len());
        for needle in needles {
            let what = &line[needle.start..needle.end];
            let content = if colour {
                Colour::Red.paint(what).to_string()
            } else {
                what.to_string()
            };
            output.push(format!("{}{}", prefix, content));
        }
        // NOTE: Use `\n` as NL
        // See https://doc.rust-lang.org/std/macro.println.html
        //    Prints to the standard output, with a newline.
        //    On all platforms, the newline is the LINE FEED character (\n/U+000A) alone
        //    (no additional CARRIAGE RETURN (\r/U+000D)).
        output.join("\n")
    }

    fn rich_format(
        &self,
        width: usize,
        prefix: &str,
        line: &str,
        needles: Vec<Range>,
        colour: bool,
    ) -> String {
        let content = if needles.is_empty() {
            line.to_string()
        } else if needles.len() == 1 {
            self.rich_format_one(width, line, &needles[0], colour)
        } else {
            self.rich_format_many(width, line, needles, colour)
        };
        format!("{}{}", prefix, content)
    }

    fn format_path(&self, path: &str, colour: bool) -> String {
        if colour {
            Colour::Blue.paint(path).to_string()
        } else {
            path.to_string()
        }
    }

    fn separator(&self, separator: &str, code: u8) -> String {
        let colour = match self {
            Format::Rich { colour, .. } => *colour,
            _ => false,
        };
        if colour {
            Colour::Fixed(code).paint(separator).to_string()
        } else {
            separator.to_string()
        }
    }
}

impl OutputFormat for Format {
    fn format(&self, width: usize, path: &str, context: Option<DisplayContext>) -> String {
        match self {
            Format::Rich {
                colour,
                match_only,
                no_path,
                no_lno,
            } => match context {
                Some(ctx) => {
                    let prefix = if *no_path {
                        "".into()
                    } else {
                        #[allow(clippy::collapsible_else_if)]
                        if *colour {
                            format!(
                                "{}{}",
                                Colour::Blue.paint(path),
                                Colour::Cyan.paint(ctx.lno_sep)
                            )
                        } else {
                            format!("{}{}", path, ctx.lno_sep)
                        }
                    };
                    let prefix = if *no_lno {
                        prefix
                    } else {
                        let lno = ctx.lno.to_string();
                        if *colour {
                            format!(
                                "{}{}{}",
                                prefix,
                                Colour::Green.paint(lno),
                                Colour::Cyan.paint(ctx.lno_sep)
                            )
                        } else {
                            format!("{}{}{}", prefix, ctx.lno, ctx.lno_sep)
                        }
                    };
                    let prefix = if prefix.is_empty() {
                        prefix
                    } else {
                        format!("{} ", prefix)
                    };
                    let needles = ctx.needle.into_iter().map(Into::into).collect();
                    if *match_only {
                        self.rich_format_needles_only(&prefix, &ctx.line, needles, *colour)
                    } else {
                        self.rich_format(width - prefix.len(), &prefix, &ctx.line, needles, *colour)
                    }
                }
                None => self.format_path(path, *colour),
            },
            Format::PathOnly { colour } => self.format_path(path, *colour),
        }
    }

    fn file_separator(&self) -> String {
        self.separator("--", 203)
    }

    fn match_separator(&self) -> String {
        self.separator("..", 120)
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
                    Some(DisplayContext::new(0, "-".repeat(len), vec![needle.into()]))
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
