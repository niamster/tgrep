use std::{
    fs::File,
    io::{self, BufRead},
    path::PathBuf,
};

use log::{debug, warn};
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
        panic!("not supported");
    }

    fn path(&self) -> &PathBuf {
        &self.path
    }
}
