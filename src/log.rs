use std::io::Write;
use std::sync::Mutex;

static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);
static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

pub fn open(path: &std::path::Path) {
    if let Ok(file) = std::fs::File::create(path) {
        *FILE.lock().unwrap_or_else(|e| e.into_inner()) = Some(file);
    }
    START.get_or_init(std::time::Instant::now);
}

pub fn write(line: &str) {
    let mut guard = FILE.lock().unwrap_or_else(|e| e.into_inner());
    let elapsed = START.get_or_init(std::time::Instant::now).elapsed();
    let text = format!("[{:>9.3}] {line}", elapsed.as_secs_f64());
    if let Some(file) = guard.as_mut() {
        let _ = writeln!(file, "{text}");
        let _ = file.flush();
    } else {
        eprintln!("{text}");
    }
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(&format!($($arg)*)) };
}
