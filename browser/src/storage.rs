//! Where the browser keeps its files, and how it writes them.
//!
//! Everything lives in `%APPDATA%\Ferrous`: the window placement, the sites
//! allowed past the blocker, the last session and the history. The files are
//! small text formats, so they need no extra dependency and stay readable if
//! someone opens them.

use std::path::PathBuf;

/// `%APPDATA%\Ferrous\<name>`, or `None` if `APPDATA` is not set.
pub fn path(name: &str) -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(base).join("Ferrous").join(name))
}

/// Read a file, treating "missing" and "unreadable" alike: callers fall back to
/// an empty default, which is right for a first run.
pub fn read(name: &str) -> Option<String> {
    std::fs::read_to_string(path(name)?).ok()
}

/// Write a file through a temporary and a rename, so a crash mid-write leaves
/// the previous version intact rather than a truncated one.
pub fn write(name: &str, contents: &str) -> std::io::Result<()> {
    let Some(path) = path(name) else {
        return Ok(());
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, contents)?;
    std::fs::rename(&temp, &path)
}
