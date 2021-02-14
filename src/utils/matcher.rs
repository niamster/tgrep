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

impl From<std::ops::Range<usize>> for Match {
    fn from(range: std::ops::Range<usize>) -> Self {
        Match {
            start: range.start,
            end: range.end,
        }
    }
}

impl Into<std::ops::Range<usize>> for Match {
    fn into(self) -> std::ops::Range<usize> {
        std::ops::Range {
            start: self.start(),
            end: self.end(),
        }
    }
}
