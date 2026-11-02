//! JSON persistence shared by profiles and preferences.
//!
//! Reads distinguish a missing file (first run) from unreadable or malformed data
//! (an error, never an implicit reset). Updates stage a clone, write a uniquely
//! named sibling file, then replace the destination before publishing memory.
//! This keeps a failed save from changing the running app's state. It does not
//! coordinate multiple app instances: the last successful replacement wins.

pub(crate) mod preferences;
pub(crate) mod profiles;

use crate::ui::locale::tr;
use anyhow::{Context, Result};
use serde::{Serialize, de::DeserializeOwned};
use std::{fs, io::Write, os::windows::ffi::OsStrExt, path::Path};
use uuid::Uuid;
use windows::{
    Win32::Storage::FileSystem::{
        MOVE_FILE_FLAGS, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    },
    core::PCWSTR,
};

/// Call the Windows replacement API using owned, NUL-terminated UTF-16 paths.
fn replace_file(from: &Path, to: &Path, flags: MOVE_FILE_FLAGS) -> windows::core::Result<()> {
    let wide = |p: &Path| {
        p.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let src = wide(from);
    let dst = wide(to);
    // SAFETY: Both buffers remain alive for the synchronous call and end in NUL.
    unsafe { MoveFileExW(PCWSTR(src.as_ptr()), PCWSTR(dst.as_ptr()), flags) }
}

/// Only `NotFound` means absence; invalid JSON and all other I/O errors propagate.
fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Replace a JSON document without exposing a partially written destination.
///
/// The temporary file lives beside the destination, so replacement stays on the
/// same volume. Flush its contents before `MoveFileExW` with write-through; report
/// write/replacement errors and attempt cleanup of the temporary file on failure.
pub(crate) fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let data = serde_json::to_vec_pretty(value)?;
    let parent = path.parent().context(tr("error.profile_path"))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!("connections-{}.tmp", Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    // Only remove a temporary file we actually created.
    let result = (|| -> Result<()> {
        file.write_all(&data)?;
        file.sync_all()?;
        drop(file);
        replace_file(
            &temp,
            path,
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Commit a cloned edit to disk, then make the same state visible to callers.
///
/// Both an edit error and a persistence error leave `current` untouched. Keep
/// external side effects out of `edit`: only this in-memory value is transactional.
fn update_json<T: Clone + Serialize, R>(
    path: &Path,
    current: &mut T,
    edit: impl FnOnce(&mut T) -> Result<R>,
) -> Result<R> {
    let mut next = current.clone();
    let result = edit(&mut next)?;
    write_json(path, &next)?;
    *current = next;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_write_keeps_previous_state() {
        let path = std::env::temp_dir().join(Uuid::new_v4().to_string());
        fs::create_dir(&path).unwrap(); // A directory cannot be replaced by a settings file.
        let mut state = 1;
        assert!(
            update_json(&path, &mut state, |n| {
                *n = 2;
                Ok(())
            })
            .is_err()
        );
        assert_eq!(state, 1);
        fs::remove_dir(path).unwrap();
    }
}
