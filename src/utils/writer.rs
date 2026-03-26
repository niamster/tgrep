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
    lines: Arc<Mutex<RefCell<Option<Vec<String>>>>>,
}

impl BufferedWriter {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        BufferedWriter {
            lines: Arc::new(Mutex::new(RefCell::new(None))),
        }
    }

    pub fn flush(&self, writer: &Arc<dyn Writer>) {
        let lines = self.lines.lock().unwrap();
        let lines = lines.borrow();
        if let Some(lines) = &*lines {
            for line in lines.iter() {
                writer.write(line);
            }
        }
    }

    pub fn has_some(&self) -> bool {
        self.lines
            .lock()
            .unwrap()
            .borrow()
            .as_ref()
            .is_some_and(|lines| !lines.is_empty())
    }
}

impl Writer for BufferedWriter {
    fn write(&self, content: &str) {
        let lines = self.lines.lock().unwrap();
        let mut lines = lines.borrow_mut();
        lines.get_or_insert_with(Vec::new).push(content.to_owned());
    }
}
