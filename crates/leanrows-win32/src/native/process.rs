//! Process-wide helpers: one window per executable path, and working-set trim.
//!
//! The first ordinary launch owns a named mutex for its lifetime. A later
//! launch hands its files to that window with `WM_COPYDATA` and exits, so every
//! open file shares one process. An installed copy and a portable copy use
//! different mutex names and never take each other's files.

use std::ffi::{OsString, c_void};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CharUpperBuffW, FindWindowExW, GetWindowThreadProcessId,
    SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_COPYDATA,
};
use windows::core::{BOOL, PCWSTR};

/// `COPYDATASTRUCT::dwData` for a list of files to open ("LRW1").
const OPEN_FILES_TAG: usize = 0x4C52_5731;
/// Paths accepted from one `WM_COPYDATA` message.
pub(super) const MAX_FORWARDED_PATHS: usize = 64;
/// Longest Win32 path in UTF-16 units, excluding the terminator.
const MAX_PATH_UNITS: usize = 32_767;
const MAX_PAYLOAD_BYTES: usize = MAX_FORWARDED_PATHS * (MAX_PATH_UNITS + 1) * 2;
const FORWARD_DEADLINE: Duration = Duration::from_secs(5);
const FORWARD_RETRY: Duration = Duration::from_millis(50);
const SEND_TIMEOUT_MS: u32 = 5_000;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_ABANDONED: u32 = 0x80;
const WAIT_TIMEOUT: u32 = 0x102;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

/// `COPYDATASTRUCT`. The `windows` crate keeps it behind a feature this crate
/// does not otherwise need, so it is declared here with the same layout.
#[repr(C)]
struct CopyData {
    tag: usize,
    byte_count: u32,
    data: *mut c_void,
}

// These kernel32 entry points sit behind `windows` crate features this crate
// does not otherwise use. Declaring the stable exports locally keeps the
// crate's Windows feature surface unchanged, as `progress.rs` does for comctl32.
#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "CreateMutexW"]
    fn create_mutex(attributes: *const c_void, initial_owner: BOOL, name: PCWSTR) -> HANDLE;
    #[link_name = "WaitForSingleObject"]
    fn wait_for_single_object(handle: HANDLE, milliseconds: u32) -> u32;
    #[link_name = "ReleaseMutex"]
    fn release_mutex(handle: HANDLE) -> BOOL;
    #[link_name = "OpenProcess"]
    fn open_process(access: u32, inherit: BOOL, process_id: u32) -> HANDLE;
    #[link_name = "QueryFullProcessImageNameW"]
    fn query_full_process_image_name(
        process: HANDLE,
        flags: u32,
        name: *mut u16,
        size: *mut u32,
    ) -> BOOL;
    #[link_name = "GetCurrentProcess"]
    safe fn current_process() -> HANDLE;
    #[link_name = "GetCurrentProcessId"]
    safe fn current_process_id() -> u32;
    #[link_name = "SetProcessWorkingSetSize"]
    fn set_process_working_set_size(process: HANDLE, minimum: usize, maximum: usize) -> BOOL;
}

/// How this launch should continue.
pub(super) enum Startup {
    /// Show the window. The guard, when present, keeps other launches
    /// forwarding here until it is dropped after the message loop.
    Primary(Option<InstanceGuard>),
    /// A running window accepted the files; this process should exit.
    Forwarded,
}

/// Owns the reader mutex. It must be dropped on the thread that claimed it.
pub(super) struct InstanceGuard(HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        // SAFETY: `claim` acquired this mutex on the current thread and this
        // guard is its only owner.
        unsafe {
            let _ = release_mutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

/// Becomes the reader for this executable, or forwards `paths` to the one that
/// is already running. If no window answers within five seconds this launch
/// opens its own window rather than lose the files.
pub(super) fn claim(paths: &[PathBuf]) -> Startup {
    let Some(executable) = process_image(current_process()).map(fold_case) else {
        return Startup::Primary(None);
    };
    let name = mutex_name(&executable);
    // SAFETY: default security, no initial owner, and a NUL-terminated name
    // that outlives the call.
    let mutex = unsafe { create_mutex(std::ptr::null(), BOOL(0), PCWSTR(name.as_ptr())) };
    if mutex.is_invalid() {
        return Startup::Primary(None);
    }
    let payloads = forward_payloads(paths);
    let deadline = Instant::now() + FORWARD_DEADLINE;
    loop {
        // SAFETY: `mutex` is a live handle owned by this function.
        match unsafe { wait_for_single_object(mutex, 0) } {
            WAIT_OBJECT_0 | WAIT_ABANDONED => return Startup::Primary(Some(InstanceGuard(mutex))),
            WAIT_TIMEOUT => {}
            _ => {
                close(mutex);
                return Startup::Primary(None);
            }
        }
        if let Some(window) = running_reader(&executable)
            && forward(window, &payloads)
        {
            close(mutex);
            return Startup::Forwarded;
        }
        if Instant::now() >= deadline {
            close(mutex);
            return Startup::Primary(None);
        }
        thread::sleep(FORWARD_RETRY);
    }
}

/// Reads the paths from a `WM_COPYDATA` open request. Returns `None` when the
/// message is not one.
pub(super) fn read_open_request(lparam: LPARAM) -> Option<Vec<PathBuf>> {
    // SAFETY: WM_COPYDATA supplies a COPYDATASTRUCT that stays valid for the
    // duration of the message; a null pointer is rejected by `as_ref`.
    let data = unsafe { (lparam.0 as *const CopyData).as_ref() }?;
    let byte_count = usize::try_from(data.byte_count).ok()?;
    if data.tag != OPEN_FILES_TAG
        || byte_count % 2 != 0
        || byte_count > MAX_PAYLOAD_BYTES
        || (byte_count != 0 && data.data.is_null())
    {
        return None;
    }
    if byte_count == 0 {
        return Some(Vec::new());
    }
    // SAFETY: the system copied exactly `byte_count` bytes into this process
    // for the duration of the message. Reading bytes avoids any assumption
    // about the alignment of that copy.
    let bytes =
        unsafe { std::slice::from_raw_parts(data.data.cast::<u8>().cast_const(), byte_count) };
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    Some(split_paths(&units))
}

/// Asks Windows to page out as much of this process as it can, the way it
/// would under memory pressure. Pages come back on demand when the window is
/// used again.
pub(super) fn trim_working_set() {
    // SAFETY: the pseudo handle for this process needs no cleanup, and passing
    // usize::MAX for both bounds is the documented request to trim.
    let _ = unsafe { set_process_working_set_size(current_process(), usize::MAX, usize::MAX) };
}

/// Upper-cases a path with the same rules Windows uses for file names.
pub(super) fn fold_case(mut units: Vec<u16>) -> Vec<u16> {
    if !units.is_empty() {
        // SAFETY: the buffer is writable for its whole length.
        let _ = unsafe { CharUpperBuffW(&mut units) };
    }
    units
}

/// Returns true when two paths name the same file, ignoring case.
pub(super) fn same_path(left: &Path, right: &Path) -> bool {
    let left: Vec<u16> = left.as_os_str().encode_wide().collect();
    let right: Vec<u16> = right.as_os_str().encode_wide().collect();
    left.len() == right.len() && fold_case(left) == fold_case(right)
}

fn close(handle: HANDLE) {
    // SAFETY: callers pass a handle they own and do not use afterwards.
    let _ = unsafe { CloseHandle(handle) };
}

fn process_image(process: HANDLE) -> Option<Vec<u16>> {
    let mut buffer = vec![0_u16; MAX_PATH_UNITS + 1];
    let mut length = u32::try_from(buffer.len()).ok()?;
    // SAFETY: the buffer is writable for `length` units and `length` is
    // writable local storage.
    let queried =
        unsafe { query_full_process_image_name(process, 0, buffer.as_mut_ptr(), &raw mut length) };
    if !queried.as_bool() {
        return None;
    }
    buffer.truncate(usize::try_from(length).ok()?);
    Some(buffer)
}

fn mutex_name(folded_executable: &[u16]) -> Vec<u16> {
    let mut name: Vec<u16> = format!("Local\\LeanRows.Reader.{:016x}", fnv1a(folded_executable))
        .encode_utf16()
        .collect();
    name.push(0);
    name
}

fn fnv1a(units: &[u16]) -> u64 {
    units.iter().fold(FNV_OFFSET, |hash, unit| {
        (hash ^ u64::from(*unit)).wrapping_mul(FNV_PRIME)
    })
}

fn running_reader(folded_executable: &[u16]) -> Option<HWND> {
    let own_process = current_process_id();
    let mut after = None;
    loop {
        // SAFETY: the class name is static and `after` is a top-level window
        // returned by the previous call. Not finding one ends the search.
        let window =
            unsafe { FindWindowExW(None, after, super::WINDOW_CLASS_NAME, PCWSTR::null()) }.ok()?;
        let mut process_id = 0_u32;
        // SAFETY: `window` came from FindWindowExW and process_id is writable.
        unsafe { GetWindowThreadProcessId(window, Some(&raw mut process_id)) };
        if process_id != 0
            && process_id != own_process
            && is_same_executable(process_id, folded_executable)
        {
            return Some(window);
        }
        after = Some(window);
    }
}

fn is_same_executable(process_id: u32, folded_executable: &[u16]) -> bool {
    // SAFETY: limited query access to a process id the window manager reported.
    let process = unsafe { open_process(PROCESS_QUERY_LIMITED_INFORMATION, BOOL(0), process_id) };
    if process.is_invalid() {
        return false;
    }
    let image = process_image(process);
    close(process);
    image.is_some_and(|image| fold_case(image) == folded_executable)
}

fn forward(window: HWND, payloads: &[Vec<u16>]) -> bool {
    let mut process_id = 0_u32;
    // SAFETY: `window` is a top-level window and process_id is writable.
    unsafe { GetWindowThreadProcessId(window, Some(&raw mut process_id)) };
    // The user started this process, so it may pass its right to take the
    // foreground on; without it Windows would only flash the taskbar button.
    // SAFETY: the call takes a process id and no pointers.
    let _ = unsafe { AllowSetForegroundWindow(process_id) };
    if payloads.is_empty() {
        return send(window, &[]);
    }
    payloads.iter().all(|payload| send(window, payload))
}

fn send(window: HWND, payload: &[u16]) -> bool {
    let Ok(byte_count) = u32::try_from(payload.len().saturating_mul(2)) else {
        return false;
    };
    let data = CopyData {
        tag: OPEN_FILES_TAG,
        byte_count,
        data: if payload.is_empty() {
            std::ptr::null_mut()
        } else {
            payload.as_ptr().cast_mut().cast()
        },
    };
    let mut accepted = 0_usize;
    // SAFETY: `data` and `payload` outlive this synchronous call, and the
    // system copies the bytes into the receiving process.
    let sent = unsafe {
        SendMessageTimeoutW(
            window,
            WM_COPYDATA,
            WPARAM(0),
            LPARAM((&raw const data) as isize),
            SMTO_ABORTIFHUNG,
            SEND_TIMEOUT_MS,
            Some(&raw mut accepted),
        )
    };
    sent.0 != 0 && accepted == 1
}

/// Encodes absolute paths as NUL-terminated UTF-16, at most
/// `MAX_FORWARDED_PATHS` per message. Relative paths are resolved here because
/// the running window has a different working directory.
fn forward_payloads(paths: &[PathBuf]) -> Vec<Vec<u16>> {
    paths
        .chunks(MAX_FORWARDED_PATHS)
        .map(|chunk| {
            let mut payload = Vec::new();
            for path in chunk {
                let absolute = std::path::absolute(path).unwrap_or_else(|_| path.clone());
                payload.extend(absolute.as_os_str().encode_wide().filter(|unit| *unit != 0));
                payload.push(0);
            }
            payload
        })
        .collect()
}

fn split_paths(units: &[u16]) -> Vec<PathBuf> {
    units
        .split(|unit| *unit == 0)
        .filter(|path| !path.is_empty() && path.len() <= MAX_PATH_UNITS)
        .take(MAX_FORWARDED_PATHS)
        .map(|path| PathBuf::from(OsString::from_wide(path)))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{
        MAX_FORWARDED_PATHS, MAX_PATH_UNITS, fnv1a, forward_payloads, mutex_name, same_path,
        split_paths,
    };

    #[test]
    fn mutex_name_uses_fnv1a_of_the_folded_executable_path() {
        assert_eq!(fnv1a(&[]), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(&[u16::from(b'a')]), 0xaf63_dc4c_8601_ec8c);
        let name = mutex_name(&[u16::from(b'A')]);
        assert_eq!(name.last(), Some(&0));
        assert_eq!(
            String::from_utf16_lossy(&name[..name.len() - 1]),
            format!("Local\\LeanRows.Reader.{:016x}", fnv1a(&[u16::from(b'A')]))
        );
    }

    #[test]
    fn payloads_round_trip_and_split_into_bounded_messages() {
        let paths: Vec<PathBuf> = (0..=MAX_FORWARDED_PATHS)
            .map(|index| PathBuf::from(format!(r"C:\data\file{index}.csv")))
            .collect();
        let payloads = forward_payloads(&paths);
        assert_eq!(payloads.len(), 2);
        let mut received = split_paths(&payloads[0]);
        received.extend(split_paths(&payloads[1]));
        assert_eq!(received, paths);
    }

    #[test]
    fn received_paths_skip_empty_and_oversized_entries() {
        let mut units: Vec<u16> = r"C:\a.csv".encode_utf16().collect();
        units.extend([0, 0]);
        units.extend(std::iter::repeat_n(u16::from(b'x'), MAX_PATH_UNITS + 1));
        units.push(0);
        units.extend(r"C:\b.log".encode_utf16());
        assert_eq!(
            split_paths(&units),
            vec![PathBuf::from(r"C:\a.csv"), PathBuf::from(r"C:\b.log")]
        );
        let many: Vec<u16> = std::iter::repeat_n([u16::from(b'x'), 0], MAX_FORWARDED_PATHS + 5)
            .flatten()
            .collect();
        assert_eq!(split_paths(&many).len(), MAX_FORWARDED_PATHS);
    }

    #[test]
    fn path_comparison_ignores_case_only() {
        assert!(same_path(
            Path::new(r"C:\Data\Résumé.CSV"),
            Path::new(r"c:\data\RÉSUMÉ.csv")
        ));
        assert!(!same_path(
            Path::new(r"C:\data\a.csv"),
            Path::new(r"C:\data\b.csv")
        ));
        assert!(!same_path(
            Path::new(r"C:\data\a.csv"),
            Path::new(r"C:\data\a.csv2")
        ));
    }
}
