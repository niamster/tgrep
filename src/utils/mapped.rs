use std::{fs, ops, path::PathBuf, rc::Rc, str};

use log::debug;
use memchr::memchr;
use memmap2::{Mmap, MmapOptions};
use streaming_iterator::StreamingIterator;

use crate::utils::lines::{LineIterator, LinesReader};

struct MappedInner {
    path: PathBuf,
    mmap: Mmap,
}

pub struct Mapped {
    mapped: Rc<MappedInner>,
}

impl Mapped {
    pub fn new(path: &PathBuf, len: usize) -> anyhow::Result<Self> {
        let file = fs::File::open(path)?;
        let mmap = unsafe { MmapOptions::new().len(len).map(&file)? };
        Ok(Mapped {
            mapped: Rc::new(MappedInner {
                path: path.to_owned(),
                mmap,
            }),
        })
    }
}

impl ops::Deref for Mapped {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &[u8] {
        &*self.mapped.mmap
    }
}

impl LinesReader for Mapped {
    fn map(&self) -> anyhow::Result<&str> {
        Ok(unsafe { str::from_utf8_unchecked(&*self) })
    }

    fn lines(&self) -> anyhow::Result<Box<LineIterator>> {
        Ok(Box::new(MappedLines::new(self.mapped.clone())?))
    }

    fn path(&self) -> &PathBuf {
        &self.mapped.path
    }
}

struct MappedLines {
    mapped: Rc<MappedInner>,
    line: ops::Range<usize>,
    pos: usize,
    buf: String,
}

impl MappedLines {
    fn new(mapped: Rc<MappedInner>) -> anyhow::Result<Self> {
        Ok(MappedLines {
            mapped,
            line: ops::Range { start: 0, end: 0 },
            pos: 0,
            buf: String::new(),
        })
    }
}

impl StreamingIterator for MappedLines {
    type Item = str;

    fn advance(&mut self) {
        let mmap = &self.mapped.mmap;
        self.line.start = self.pos;
        if self.line.start >= mmap.len() {
            return;
        }
        self.line.end = match memchr(b'\n', &mmap[self.line.start..]) {
            Some(pos) => self.line.start + pos,
            None => mmap.len(),
        };
        self.pos = self.line.end + 1;
        if self.pos < self.mapped.mmap.len() && mmap[self.pos] == b'\r' {
            self.pos += 1;
        }
        if (1..mmap.len()).contains(&self.line.end) && mmap[self.line.end] == b'\r' {
            self.line.end -= 1;
        }
    }

    fn get(&self) -> Option<&Self::Item> {
        panic!("Should not be called");
    }

    fn next(&mut self) -> Option<&Self::Item> {
        self.advance();
        if self.line.start >= self.mapped.mmap.len() {
            return None;
        }
        let line = &self.mapped.mmap[self.line.start..self.line.end];
        match str::from_utf8(line) {
            Ok(line) => Some(line),
            Err(e) => {
                self.buf = line.iter().map(|&c| c as char).collect();
                debug!(
                    "UTF-8 decoding failure of '{}' at [{};{}], transformed to '{}'",
                    self.mapped.path.display(),
                    self.line.start + e.valid_up_to(),
                    self.line.start + e.valid_up_to() + e.error_len().unwrap_or(0),
                    self.buf,
                );
                Some(&self.buf)
            }
        }
    }
}
