use std::{
    fs::File,
    io::{self, BufRead, BufReader},
    path::PathBuf,
};

pub trait ToLines {
    fn to_lines(&self) -> io::Result<io::Lines<BufReader<File>>>;
}

impl ToLines for PathBuf {
    fn to_lines(&self) -> io::Result<io::Lines<BufReader<File>>> {
        let file = File::open(self.as_path())?;
        Ok(io::BufReader::new(file).lines())
    }
}
