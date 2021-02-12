use std::sync::Arc;

#[derive(Clone)]
pub struct Match {
    start: usize,
    end: usize,
}

pub type Matcher = Arc<Box<dyn Fn(&str) -> Option<Match> + Send + Sync>>;

impl Match {
    pub fn new(start: usize, end: usize) -> Self {
        Match { start, end }
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }
}
