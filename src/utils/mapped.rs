use std::{
    fs, ops,
    path::{Path, PathBuf},
    str,
    sync::Arc,
};

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
    mapped: Arc<MappedInner>,
}

impl Mapped {
    pub fn open(path: &Path) -> anyhow::Result<Option<Self>> {
        let file = fs::File::open(path)?;
        if file.metadata()?.len() == 0 {
            return Ok(None);
        }
        let mmap = unsafe { MmapOptions::new().map(&file)? };
        Ok(Some(Mapped {
            mapped: Arc::new(MappedInner {
                path: path.to_owned(),
                mmap,
            }),
        }))
    }
}

impl ops::Deref for Mapped {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &[u8] {
        &self.mapped.mmap
    }
}

impl LinesReader for Mapped {
    fn map(&self) -> anyhow::Result<&str> {
        Ok(unsafe { str::from_utf8_unchecked(self) })
    }

    fn lines(&self) -> anyhow::Result<Box<LineIterator>> {
        Ok(Box::new(MappedLines::new(self.mapped.clone())?))
    }

    fn path(&self) -> &PathBuf {
        &self.mapped.path
    }
}

struct MappedLines {
    mapped: Arc<MappedInner>,
    line: ops::Range<usize>,
    pos: usize,
    buf: String,
}

impl MappedLines {
    fn new(mapped: Arc<MappedInner>) -> anyhow::Result<Self> {
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
        if self.line.end > self.line.start && mmap[self.line.end - 1] == b'\r' {
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    struct TempFile {
        path: PathBuf,
    }

    impl TempFile {
        fn new(contents: &[u8]) -> Self {
            let mut path = std::env::temp_dir();
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            path.push(format!("tgrep-mapped-test-{}", unique));
            fs::write(&path, contents).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    #[test]
    fn mapped_reader_returns_lines() {
        let file = TempFile::new(b"alpha\nbeta\ngamma");
        let mapped = Mapped::open(file.path()).unwrap().unwrap();
        let mut lines = mapped.lines().unwrap();

        assert_eq!(Some("alpha"), lines.next());
        assert_eq!(Some("beta"), lines.next());
        assert_eq!(Some("gamma"), lines.next());
        assert_eq!(None, lines.next());
    }

    #[test]
    fn mapped_reader_trims_cr_before_lf() {
        let file = TempFile::new(b"alpha\r\nbeta\r\n");
        let mapped = Mapped::open(file.path()).unwrap().unwrap();
        let mut lines = mapped.lines().unwrap();

        assert_eq!(Some("alpha"), lines.next());
        assert_eq!(Some("beta"), lines.next());
        assert_eq!(None, lines.next());
    }

    #[test]
    fn mapped_reader_falls_back_for_invalid_utf8() {
        let file = TempFile::new(b"ok\nf\x80o\n");
        let mapped = Mapped::open(file.path()).unwrap().unwrap();
        let mut lines = mapped.lines().unwrap();

        assert_eq!(Some("ok"), lines.next());
        assert_eq!(Some("f\u{80}o"), lines.next());
        assert_eq!(None, lines.next());
    }
}
