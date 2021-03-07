use std::{fs, io, os::unix::fs::FileTypeExt, os::unix::io::FromRawFd, path::PathBuf};

use crate::utils::lines::{LineIterator, Lines, LinesReader};

pub struct Stdin {
    file: fs::File,
    path: PathBuf,
}

impl Stdin {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let file = unsafe { fs::File::from_raw_fd(0) };
        Stdin {
            file,
            path: PathBuf::from("<stdin>"),
        }
    }

    pub fn is_readable(&self) -> bool {
        match self.file.metadata() {
            Ok(meta) => {
                let file_type = meta.file_type();
                file_type.is_file() || file_type.is_fifo() || file_type.is_socket()
            }
            Err(_) => false,
        }
    }
}

impl LinesReader for Stdin {
    fn lines(&self) -> anyhow::Result<Box<LineIterator>> {
        Ok(Box::new(Lines::new(
            io::BufReader::new(self.file.try_clone()?),
            self.path.clone(),
        )))
    }

    fn path(&self) -> &PathBuf {
        &self.path
    }
}
