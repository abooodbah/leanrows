//! One-file HDROP extraction with fixed path and count bounds.

use std::ffi::{OsString, c_void};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use crate::ShellError;

const QUERY_FILE_COUNT: u32 = u32::MAX;
const MAX_DROPPED_PATH_UTF16: usize = 32_767;

#[link(name = "shell32")]
unsafe extern "system" {
    #[link_name = "DragQueryFileW"]
    fn drag_query_file(drop: *mut c_void, index: u32, path: *mut u16, path_capacity: u32) -> u32;
    #[link_name = "DragFinish"]
    fn drag_finish(drop: *mut c_void);
}

struct DropGuard(*mut c_void);

impl Drop for DropGuard {
    fn drop(&mut self) {
        // SAFETY: WM_DROPFILES transfers this HDROP cleanup obligation once.
        unsafe { drag_finish(self.0) };
    }
}

pub(super) fn one_path(raw_drop: usize) -> Result<PathBuf, ShellError> {
    let drop = DropGuard(raw_drop as *mut c_void);
    if drop.0.is_null() {
        return Err(ShellError::new("dropped-file handle is invalid"));
    }
    // SAFETY: QUERY_FILE_COUNT requests only the item count and writes no path.
    let count = unsafe { drag_query_file(drop.0, QUERY_FILE_COUNT, std::ptr::null_mut(), 0) };
    if count != 1 {
        return Err(ShellError::new(
            "drop exactly one CSV, TSV, JSONL, NDJSON, LOG, or TXT file",
        ));
    }
    // SAFETY: A null buffer requests the selected path length excluding NUL.
    let length = unsafe { drag_query_file(drop.0, 0, std::ptr::null_mut(), 0) } as usize;
    if length == 0 || length > MAX_DROPPED_PATH_UTF16 {
        return Err(ShellError::new("dropped-file path is empty or too long"));
    }
    let capacity = length
        .checked_add(1)
        .ok_or_else(|| ShellError::new("dropped-file path length overflow"))?;
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(capacity)
        .map_err(|_| ShellError::new("dropped-file path allocation is unavailable"))?;
    buffer.resize(capacity, 0);
    let native_capacity = u32::try_from(capacity)
        .map_err(|_| ShellError::new("dropped-file path exceeds the native limit"))?;
    // SAFETY: buffer is writable for native_capacity UTF-16 units.
    let copied = unsafe { drag_query_file(drop.0, 0, buffer.as_mut_ptr(), native_capacity) };
    if copied as usize != length || buffer.get(length) != Some(&0) {
        return Err(ShellError::new("dropped-file path transfer was incomplete"));
    }
    Ok(PathBuf::from(OsString::from_wide(&buffer[..length])))
}
