// debug logging - prints to stderr only in debug builds
// in release builds this compiles to a no-op (zero overhead)
macro_rules! dbg_log {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            eprintln!("[DBG] {}", format!($($arg)*))
        }
    };
}
