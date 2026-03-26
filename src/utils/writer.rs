use std::{
    cell::RefCell,
    sync::{Arc, Mutex},
};

pub trait Writer: Send + Sync {
    fn write(&self, content: &str);
}

#[derive(Clone)]
pub struct StdoutWriter {
    lock: Arc<Mutex<()>>,
}

impl StdoutWriter {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        StdoutWriter {
            lock: Arc::new(Mutex::new(())),
        }
    }
}

impl Writer for StdoutWriter {
    fn write(&self, content: &str) {
        let guard = self.lock.lock();
        println!("{}", content);
        drop(guard);
    }
}

#[derive(Clone)]
pub struct BufferedWriter {
    lines: Arc<Mutex<RefCell<Vec<String>>>>,
}

impl BufferedWriter {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        BufferedWriter {
            lines: Arc::new(Mutex::new(RefCell::new(Vec::new()))),
        }
    }

    pub fn flush(&self, writer: &Arc<dyn Writer>) {
        let lines = self.lines.lock().unwrap();
        let lines = lines.borrow();
        for line in lines.iter() {
            writer.write(line);
        }
    }

    pub fn has_some(&self) -> bool {
        !self.lines.lock().unwrap().borrow().is_empty()
    }
}

impl Writer for BufferedWriter {
    fn write(&self, content: &str) {
        let lines = self.lines.lock().unwrap();
        let mut lines = lines.borrow_mut();
        lines.push(content.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct TestWriter {
        writes: Arc<Mutex<RefCell<Vec<String>>>>,
    }

    impl TestWriter {
        fn new() -> Self {
            Self {
                writes: Arc::new(Mutex::new(RefCell::new(Vec::new()))),
            }
        }

        fn lines(&self) -> Vec<String> {
            self.writes.lock().unwrap().borrow().clone()
        }
    }

    impl Writer for TestWriter {
        fn write(&self, content: &str) {
            self.writes.lock().unwrap().borrow_mut().push(content.to_owned());
        }
    }

    #[test]
    fn buffered_writer_tracks_presence_and_flushes_in_order() {
        let buffered = BufferedWriter::new();
        let test_writer = Arc::new(TestWriter::new());
        let writer: Arc<dyn Writer> = test_writer.clone();

        assert!(!buffered.has_some());

        buffered.write("first");
        buffered.write("second");

        assert!(buffered.has_some());
        buffered.flush(&writer);

        assert_eq!(vec!["first", "second"], test_writer.lines());
    }
}
