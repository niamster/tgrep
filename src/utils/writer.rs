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
}

impl Writer for BufferedWriter {
    fn write(&self, content: &str) {
        let lines = self.lines.lock().unwrap();
        let mut lines = lines.borrow_mut();
        lines.push(content.to_owned());
    }
}
