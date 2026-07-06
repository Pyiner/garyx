//! Crash-safe persistence for the small file-backed gateway stores
//! (custom agents / agent teams / wikis).

use std::path::{Path, PathBuf};

/// Write `json` to `path` atomically: write a sibling temp file first, then
/// rename it over the target. Readers never observe a torn file, and a crash
/// mid-write cannot destroy the previous good state.
pub(crate) fn write_json_atomic(path: &Path, json: &str) -> Result<(), String> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, json).map_err(|error| error.to_string())?;
    std::fs::rename(&tmp, path).map_err(|error| error.to_string())
}
