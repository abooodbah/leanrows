//! Bounded `CF_UNICODETEXT` transfer without adding broad Windows crate features.

use std::ffi::c_void;

use windows::Win32::Foundation::HWND;

use crate::ShellError;

const CF_UNICODETEXT: u32 = 13;
const GMEM_MOVEABLE: u32 = 2;

#[link(name = "user32")]
unsafe extern "system" {
    #[link_name = "OpenClipboard"]
    fn open_clipboard(owner: *mut c_void) -> i32;
    #[link_name = "CloseClipboard"]
    fn close_clipboard() -> i32;
    #[link_name = "EmptyClipboard"]
    fn empty_clipboard() -> i32;
    #[link_name = "SetClipboardData"]
    fn set_clipboard_data(format: u32, memory: *mut c_void) -> *mut c_void;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "GlobalAlloc"]
    fn global_alloc(flags: u32, bytes: usize) -> *mut c_void;
    #[link_name = "GlobalFree"]
    fn global_free(memory: *mut c_void) -> *mut c_void;
    #[link_name = "GlobalLock"]
    fn global_lock(memory: *mut c_void) -> *mut c_void;
    #[link_name = "GlobalUnlock"]
    fn global_unlock(memory: *mut c_void) -> i32;
}

struct GlobalMemory(*mut c_void);

impl GlobalMemory {
    fn allocate(bytes: usize) -> Result<Self, ShellError> {
        // SAFETY: The size is checked by the caller and GMEM_MOVEABLE is required
        // for clipboard ownership transfer.
        let memory = unsafe { global_alloc(GMEM_MOVEABLE, bytes) };
        if memory.is_null() {
            Err(ShellError::new("clipboard memory allocation failed"))
        } else {
            Ok(Self(memory))
        }
    }

    fn release(mut self) -> *mut c_void {
        let memory = self.0;
        self.0 = std::ptr::null_mut();
        memory
    }
}

impl Drop for GlobalMemory {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: This guard uniquely owns the still-unpublished allocation.
            let _ = unsafe { global_free(self.0) };
        }
    }
}

struct ClipboardGuard;

impl ClipboardGuard {
    fn open(owner: HWND) -> Result<Self, ShellError> {
        // SAFETY: The live top-level HWND becomes the clipboard owner only for
        // the duration of this synchronous command.
        if unsafe { open_clipboard(owner.0) } == 0 {
            Err(ShellError::new(
                "the clipboard is busy; close another clipboard operation and retry",
            ))
        } else {
            Ok(Self)
        }
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        // SAFETY: This guard exists only after OpenClipboard succeeded.
        let _ = unsafe { close_clipboard() };
    }
}

pub(super) fn set_unicode_text(owner: HWND, text: &[u16]) -> Result<(), ShellError> {
    if text.last() != Some(&0) || text[..text.len().saturating_sub(1)].contains(&0) {
        return Err(ShellError::new(
            "clipboard text was not one valid NUL-terminated value",
        ));
    }
    let bytes = text
        .len()
        .checked_mul(size_of::<u16>())
        .ok_or_else(|| ShellError::new("clipboard text size overflow"))?;
    let memory = GlobalMemory::allocate(bytes)?;
    // SAFETY: GlobalLock returns writable storage for the checked allocation.
    let destination = unsafe { global_lock(memory.0) };
    if destination.is_null() {
        return Err(ShellError::new("clipboard memory lock failed"));
    }
    // SAFETY: Both regions are valid for exactly text.len() u16 values and do not overlap.
    unsafe {
        std::ptr::copy_nonoverlapping(text.as_ptr(), destination.cast::<u16>(), text.len());
        let _ = global_unlock(memory.0);
    }

    let _clipboard = ClipboardGuard::open(owner)?;
    // SAFETY: The clipboard is open on this thread.
    if unsafe { empty_clipboard() } == 0 {
        return Err(ShellError::new("clipboard could not be cleared"));
    }
    // SAFETY: On success, the system takes ownership of the movable allocation.
    if unsafe { set_clipboard_data(CF_UNICODETEXT, memory.0) }.is_null() {
        return Err(ShellError::new("clipboard text publication failed"));
    }
    let _ = memory.release();
    Ok(())
}
