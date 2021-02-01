use std::path::PathBuf;

use ansi_term::Colour;
use regex;

pub trait Display {
    fn display(&self, path: &PathBuf, lno: u32, line: &String, needle: &regex::Match);
}

pub struct DisplayTerminal {
    margin: usize,
}

impl DisplayTerminal {
    pub fn new(margin: usize) -> Self {
        DisplayTerminal { margin }
    }
}

impl Display for DisplayTerminal {
    fn display(&self, path: &PathBuf, lno: u32, line: &String, needle: &regex::Match) {
        let (start, prefix) = if needle.start() > self.margin {
            (needle.start() - self.margin, "[...] ")
        } else {
            (0, "")
        };
        let (end, suffix) = if line.len() - needle.end() > self.margin {
            (needle.end() + self.margin, " [...]")
        } else {
            (line.len(), "")
        };
        let before = &line[start..needle.start()];
        let what = &line[needle.start()..needle.end()];
        let after = &line[needle.end()..end];
        println!(
            "{}:{} {}{}{}{}{}",
            Colour::Blue.paint(path.to_str().unwrap()),
            Colour::Green.paint(lno.to_string()),
            Colour::Purple.paint(prefix),
            before,
            Colour::Red.paint(what),
            after,
            Colour::Purple.paint(suffix),
        );
    }
}
