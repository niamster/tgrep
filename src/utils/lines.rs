use std::{
    fs::File,
    io::{self, BufRead},
    iter::Iterator,
    path::PathBuf,
};

pub trait LinesReader {
    fn lines(&self) -> io::Result<Box<dyn Iterator<Item = io::Result<String>>>>;
    fn path(&self) -> &PathBuf;
}

impl LinesReader for PathBuf {
    fn lines(&self) -> io::Result<Box<dyn Iterator<Item = io::Result<String>>>> {
        let file = File::open(self.as_path())?;
        Ok(Box::new(io::BufReader::new(file).lines()))
    }

    fn path(&self) -> &PathBuf {
        self
    }
}
