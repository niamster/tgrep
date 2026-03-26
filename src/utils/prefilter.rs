use memchr::memmem;

#[derive(Clone)]
pub struct LiteralPrefilter {
    needle: Vec<u8>,
}

impl LiteralPrefilter {
    pub fn new(needle: Vec<u8>) -> Self {
        Self { needle }
    }

    pub fn matches(&self, haystack: &[u8]) -> bool {
        memmem::find(haystack, &self.needle).is_some()
    }
}

pub fn plain_literal(pattern: &str) -> Option<LiteralPrefilter> {
    if pattern.is_empty() || pattern.bytes().any(is_regex_meta) {
        return None;
    }
    Some(LiteralPrefilter::new(pattern.as_bytes().to_vec()))
}

fn is_regex_meta(byte: u8) -> bool {
    matches!(
        byte,
        b'\\'
            | b'.'
            | b'+'
            | b'*'
            | b'?'
            | b'('
            | b')'
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b'^'
            | b'$'
            | b'|'
    )
}
