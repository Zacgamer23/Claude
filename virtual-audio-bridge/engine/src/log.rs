//! Registro de eventos para la interfaz, la consola y un archivo.
//!
//! Solo lo usan los hilos que no son de tiempo real (los hilos de audio
//! informan mediante contadores en `Stats` y solo registran al arrancar/fallar).

use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const KEEP: usize = 500;

#[derive(Clone, serde::Serialize)]
pub struct Entry {
    pub ts_ms: u64,
    pub level: &'static str,
    pub msg: String,
}

#[derive(Clone)]
pub struct Log(Arc<Inner>);

struct Inner {
    entries: Mutex<VecDeque<Entry>>,
    file: Mutex<Option<std::fs::File>>,
}

impl Log {
    pub fn new(path: Option<std::path::PathBuf>) -> Self {
        let file = path.and_then(|p| {
            std::fs::OpenOptions::new().create(true).append(true).open(p).ok()
        });
        Log(Arc::new(Inner { entries: Mutex::new(VecDeque::new()), file: Mutex::new(file) }))
    }

    fn push(&self, level: &'static str, msg: &str) {
        let ts_ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        eprintln!("[{level}] {msg}");
        if let Ok(mut f) = self.0.file.lock() {
            if let Some(f) = f.as_mut() {
                let _ = writeln!(f, "{ts_ms} [{level}] {msg}");
            }
        }
        if let Ok(mut q) = self.0.entries.lock() {
            if q.len() >= KEEP {
                q.pop_front();
            }
            q.push_back(Entry { ts_ms, level, msg: msg.to_string() });
        }
    }

    pub fn info(&self, m: &str) {
        self.push("info", m)
    }
    pub fn warn(&self, m: &str) {
        self.push("warn", m)
    }
    pub fn error(&self, m: &str) {
        self.push("error", m)
    }

    pub fn recent(&self, n: usize) -> Vec<Entry> {
        let q = self.0.entries.lock().unwrap();
        q.iter().skip(q.len().saturating_sub(n)).cloned().collect()
    }
}
