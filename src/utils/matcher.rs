use std::sync::Arc;

#[derive(Clone)]
pub struct Match {
    start: usize,
    end: usize,
}

pub enum MatcherOptions {
    Fuzzy,
    Exact(usize),
}

pub type Matcher = Arc<Box<dyn Fn(&str, MatcherOptions) -> Option<Vec<Match>> + Send + Sync>>;

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

impl From<Match> for std::ops::Range<usize> {
    fn from(m: Match) -> Self {
        std::ops::Range {
            start: m.start,
            end: m.end,
        }
    }
}
