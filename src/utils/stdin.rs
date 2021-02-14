use std::{
    fs,
    io::{self, BufRead},
    iter::Iterator,
    os::unix::fs::FileTypeExt,
    os::unix::io::FromRawFd,
    path::PathBuf,
};

use crate::utils::lines::LinesReader;

pub struct Stdin {
    file: fs::File,
    path: PathBuf,
}

impl Stdin {
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
    fn lines(&self) -> io::Result<Box<dyn Iterator<Item = io::Result<String>>>> {
        Ok(Box::new(io::BufReader::new(self.file.try_clone()?).lines()))
    }

    fn path(&self) -> &PathBuf {
        &self.path
    }
}
