use std::{
    fs::File,
    io::{self, BufRead},
    ops,
    path::PathBuf,
    str,
    sync::Arc,
};

use log::{debug, warn};
use memchr::memchr;
// See https://users.rust-lang.org/t/unconstrained-lifetime-parameter-for-impl/27995
use streaming_iterator::StreamingIterator;

pub type LineIterator = dyn StreamingIterator<Item = str>;

pub trait LinesReader {
    fn map(&self) -> anyhow::Result<&str> {
        anyhow::bail!("not supported");
    }

    fn lines(&self) -> anyhow::Result<Box<LineIterator>>;
    fn path(&self) -> &PathBuf;
}

impl LinesReader for PathBuf {
    fn lines(&self) -> anyhow::Result<Box<LineIterator>> {
        let file = File::open(self.as_path())?;
        Ok(Box::new(Lines::new(
            io::BufReader::new(file),
            self.to_path_buf(),
        )))
    }

    fn path(&self) -> &PathBuf {
        self
    }
}

pub struct Lines<T> {
    reader: T,
    path: PathBuf,
    buf: String,
    end: bool,
}

impl<T> Lines<T> {
    pub fn new(reader: T, path: PathBuf) -> Self {
        Lines {
            reader,
            path,
            buf: String::new(),
            end: false,
        }
    }
}

impl<T> StreamingIterator for Lines<T>
where
    T: BufRead,
{
    type Item = str;

    fn advance(&mut self) {
        self.buf.clear();
        match self.reader.read_line(&mut self.buf) {
            Ok(0) => {
                self.end = true;
            }
            Ok(_) => {
                if self.buf.ends_with('\n') {
                    self.buf.pop();
                    if self.buf.ends_with('\r') {
                        self.buf.pop();
                    }
                }
            }
            Err(e) => {
                match e.kind() {
                    std::io::ErrorKind::InvalidData => {
                        // Likely some non-unicode encoding
                        debug!("Failed to read '{}': {}", self.path.display(), e);
                    }
                    _ => {
                        self.end = true;
                        warn!("Failed to read '{}': {}", self.path.display(), e);
                    }
                }
            }
        };
    }

    fn get(&self) -> Option<&Self::Item> {
        if self.end {
            None
        } else {
            Some(&self.buf)
        }
    }
}

struct OwnedLinesInner {
    path: PathBuf,
    content: Vec<u8>,
}

#[derive(Clone)]
pub struct OwnedLinesReader {
    inner: Arc<OwnedLinesInner>,
}

impl OwnedLinesReader {
    pub fn new(path: PathBuf, content: Vec<u8>) -> Self {
        Self {
            inner: Arc::new(OwnedLinesInner { path, content }),
        }
    }
}

impl LinesReader for OwnedLinesReader {
    fn map(&self) -> anyhow::Result<&str> {
        str::from_utf8(&self.inner.content).map_err(anyhow::Error::new)
    }

    fn lines(&self) -> anyhow::Result<Box<LineIterator>> {
        Ok(Box::new(OwnedLines::new(self.inner.clone())))
    }

    fn path(&self) -> &PathBuf {
        &self.inner.path
    }
}

struct OwnedLines {
    inner: Arc<OwnedLinesInner>,
    line: ops::Range<usize>,
    pos: usize,
    buf: String,
}

impl OwnedLines {
    fn new(inner: Arc<OwnedLinesInner>) -> Self {
        Self {
            inner,
            line: ops::Range { start: 0, end: 0 },
            pos: 0,
            buf: String::new(),
        }
    }
}

impl StreamingIterator for OwnedLines {
    type Item = str;

    fn advance(&mut self) {
        let content = &self.inner.content;
        self.line.start = self.pos;
        if self.line.start >= content.len() {
            return;
        }
        self.line.end = match memchr(b'\n', &content[self.line.start..]) {
            Some(pos) => self.line.start + pos,
            None => content.len(),
        };
        self.pos = self.line.end + 1;
        if self.line.end > self.line.start && content[self.line.end - 1] == b'\r' {
            self.line.end -= 1;
        }
    }

    fn get(&self) -> Option<&Self::Item> {
        panic!("Should not be called");
    }

    fn next(&mut self) -> Option<&Self::Item> {
        self.advance();
        if self.line.start >= self.inner.content.len() {
            return None;
        }
        let line = &self.inner.content[self.line.start..self.line.end];
        match str::from_utf8(line) {
            Ok(line) => Some(line),
            Err(e) => {
                self.buf = line.iter().map(|&c| c as char).collect();
                debug!(
                    "UTF-8 decoding failure of '{}' at [{};{}], transformed to '{}'",
                    self.inner.path.display(),
                    self.line.start + e.valid_up_to(),
                    self.line.start + e.valid_up_to() + e.error_len().unwrap_or(0),
                    self.buf,
                );
                Some(&self.buf)
            }
        }
    }
}

#[derive(Clone, PartialOrd, PartialEq, Ord, Eq)]
pub struct Zero {
    path: PathBuf,
}

impl Zero {
    pub fn new(path: PathBuf) -> Self {
        Zero { path }
    }
}

impl LinesReader for Zero {
    fn map(&self) -> anyhow::Result<&str> {
        Ok("")
    }

    fn lines(&self) -> anyhow::Result<Box<LineIterator>> {
        Ok(Box::new(self.clone()))
    }

    fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl StreamingIterator for Zero {
    type Item = str;

    fn advance(&mut self) {}

    fn get(&self) -> Option<&Self::Item> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use streaming_iterator::StreamingIterator;

    use super::*;

    #[test]
    fn lines_trim_lf_and_crlf_endings() {
        let mut lines = Lines::new(
            Cursor::new(b"alpha\nbeta\r\ngamma".to_vec()),
            PathBuf::from("input.txt"),
        );

        assert_eq!(Some("alpha"), lines.next());
        assert_eq!(Some("beta"), lines.next());
        assert_eq!(Some("gamma"), lines.next());
        assert_eq!(None, lines.next());
    }

    #[test]
    fn zero_reader_maps_to_empty_and_has_no_lines() {
        let zero = Zero::new(PathBuf::from("empty.txt"));

        assert_eq!("", LinesReader::map(&zero).unwrap());

        let mut lines = zero.lines().unwrap();
        assert_eq!(None, lines.next());
    }

    #[test]
    fn owned_lines_reader_maps_and_iterates() {
        let reader =
            OwnedLinesReader::new(PathBuf::from("input.txt"), b"alpha\nbeta\r\ngamma".to_vec());

        assert_eq!("alpha\nbeta\r\ngamma", LinesReader::map(&reader).unwrap());

        let mut lines = reader.lines().unwrap();
        assert_eq!(Some("alpha"), lines.next());
        assert_eq!(Some("beta"), lines.next());
        assert_eq!(Some("gamma"), lines.next());
        assert_eq!(None, lines.next());
    }
}
