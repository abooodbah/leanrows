//! LeanMark-aligned painting for the native progress control.
//!
//! `LeanRows` retains the common-control progress HWND so Windows continues to
//! expose its native progress-bar role and value to assistive technology. Only
//! the pixels are replaced: a flat, two-DIP rule uses the current surface and
//! border behind an accent prefix, matching the corresponding `LeanMark` reader
//! treatment without native chunks, glow, or a marquee.

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, FillRect, HDC, InvalidateRect, PAINTSTRUCT,
};
use windows::Win32::UI::Controls::{PBM_GETPOS, PBM_SETPOS};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, SendMessageW, WM_ERASEBKGND, WM_NCDESTROY, WM_PAINT, WM_PRINTCLIENT,
};
use windows::core::BOOL;

use crate::ShellError;

const PROGRESS_SUBCLASS_ID: usize = 0x4c52_5047;
const PROGRESS_MAXIMUM: u64 = 100;

type SubclassProcedure =
    Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT>;

// The `windows` crate currently locates these common-control helpers under its
// optional `Win32_UI_Shell` feature. Declaring the three stable Comctl32 entry
// points locally keeps this small paint module within the crate's existing
// Windows feature surface.
#[link(name = "comctl32")]
unsafe extern "system" {
    #[link_name = "SetWindowSubclass"]
    fn set_window_subclass(
        window: HWND,
        procedure: SubclassProcedure,
        subclass_id: usize,
        reference_data: usize,
    ) -> BOOL;

    #[link_name = "RemoveWindowSubclass"]
    fn remove_window_subclass(
        window: HWND,
        procedure: SubclassProcedure,
        subclass_id: usize,
    ) -> BOOL;

    #[link_name = "DefSubclassProc"]
    fn default_subclass_procedure(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT;
}

/// Installs the pixel-only paint override on an existing progress-bar HWND.
///
/// The control must remain a `PROGRESS_CLASSW` child of `parent`; its native
/// range and position continue to be the semantic accessibility source.
pub(super) fn install_subclass(progress: HWND, parent: HWND) -> Result<(), ShellError> {
    if progress.0.is_null() || parent.0.is_null() {
        return Err(ShellError::new(
            "progress paint subclass requires live child and parent windows",
        ));
    }
    // SAFETY: both HWNDs are live on the UI thread. The callback and identifier
    // are static, and reference_data contains only the parent HWND value.
    let installed = unsafe {
        set_window_subclass(
            progress,
            Some(progress_subclass_procedure),
            PROGRESS_SUBCLASS_ID,
            parent.0 as usize,
        )
    };
    if !installed.as_bool() {
        return Err(ShellError::new(
            "native progress paint subclass installation failed",
        ));
    }
    invalidate(progress);
    Ok(())
}

/// Updates the native semantic value and schedules the matching flat repaint.
pub(super) fn update_position(progress: HWND, percent: u64) {
    if progress.0.is_null() {
        return;
    }
    let percent = percent.min(PROGRESS_MAXIMUM);
    let position = usize::try_from(percent).unwrap_or(100);
    // SAFETY: PBM_SETPOS carries a bounded integer and no pointers. Keeping the
    // native position current preserves the common control's accessible value.
    unsafe {
        SendMessageW(
            progress,
            PBM_SETPOS,
            Some(WPARAM(position)),
            Some(LPARAM(0)),
        );
    }
    invalidate(progress);
}

/// Invalidates only the hairline; callers can use this after a theme/DPI swap.
pub(super) fn invalidate(progress: HWND) {
    if progress.0.is_null() {
        return;
    }
    // SAFETY: the live child HWND owns its update region; erasing is unnecessary
    // because the custom paint always covers the complete client rectangle.
    let _ = unsafe { InvalidateRect(Some(progress), None, false) };
}

unsafe extern "system" fn progress_subclass_procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    reference_data: usize,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => paint_window(window, reference_data),
        WM_PRINTCLIENT => {
            let device = HDC(wparam.0 as *mut c_void);
            if device.0.is_null() || !paint_client(window, reference_data, device) {
                // SAFETY: messages not paintable by this bounded override remain
                // the responsibility of the original progress procedure.
                unsafe { default_subclass_procedure(window, message, wparam, lparam) }
            } else {
                LRESULT(0)
            }
        }
        WM_NCDESTROY => {
            // SAFETY: the callback/id pair exactly matches installation. Removal
            // precedes default non-client teardown so it cannot be called again.
            let _ = unsafe {
                remove_window_subclass(
                    window,
                    Some(progress_subclass_procedure),
                    PROGRESS_SUBCLASS_ID,
                )
            };
            // SAFETY: common-control teardown must continue after our removal.
            unsafe { default_subclass_procedure(window, message, wparam, lparam) }
        }
        _ => {
            // SAFETY: all behavior except painting is intentionally retained,
            // including PBM range/value handling and accessibility semantics.
            unsafe { default_subclass_procedure(window, message, wparam, lparam) }
        }
    }
}

fn paint_window(window: HWND, reference_data: usize) -> LRESULT {
    let mut client = RECT::default();
    // SAFETY: the progress HWND is live for the duration of this callback and
    // the rectangle is writable local storage.
    if unsafe { GetClientRect(window, &raw mut client) }.is_err() {
        // A failed geometry query leaves the original control responsible for
        // validating and painting its update region.
        // SAFETY: this is the documented subclass fallback path.
        return unsafe { default_subclass_procedure(window, WM_PAINT, WPARAM(0), LPARAM(0)) };
    }

    let parent = parent_from_reference(reference_data);
    let state_pointer = super::state_pointer_for(parent);
    // SAFETY: the parent owns the child and retains its attached WindowState
    // until after child HWND destruction completes.
    if unsafe { state_pointer.as_ref() }.is_none() {
        // SAFETY: this is the documented subclass fallback path.
        return unsafe { default_subclass_procedure(window, WM_PAINT, WPARAM(0), LPARAM(0)) };
    }

    let mut paint = PAINTSTRUCT::default();
    // SAFETY: WM_PAINT supplies a live HWND and paint is writable for BeginPaint.
    let device = unsafe { BeginPaint(window, &raw mut paint) };
    if !device.0.is_null() {
        let _ = paint_client_rect(window, state_pointer, device, client);
    }
    // SAFETY: every successful BeginPaint pairing ends synchronously here.
    let _ = unsafe { EndPaint(window, &raw const paint) };
    LRESULT(0)
}

fn paint_client(window: HWND, reference_data: usize, device: HDC) -> bool {
    let mut client = RECT::default();
    // SAFETY: window is live and client is writable local storage.
    if unsafe { GetClientRect(window, &raw mut client) }.is_err() {
        return false;
    }
    let parent = parent_from_reference(reference_data);
    let state_pointer = super::state_pointer_for(parent);
    // SAFETY: the parent owns this child and retains its attached state through
    // synchronous WM_PRINTCLIENT dispatch.
    if unsafe { state_pointer.as_ref() }.is_none() {
        return false;
    }
    paint_client_rect(window, state_pointer, device, client)
}

fn paint_client_rect(
    window: HWND,
    state_pointer: *mut super::WindowState,
    device: HDC,
    client: RECT,
) -> bool {
    // SAFETY: callers validated the parent-attached pointer for the duration of
    // this synchronous paint operation.
    let Some(state) = (unsafe { state_pointer.as_ref() }) else {
        return false;
    };
    let width = client.right.saturating_sub(client.left).max(0);
    let height = client.bottom.saturating_sub(client.top).max(0);
    let (surface_height, border_height) =
        background_split(height, state.theme.metrics.border_width);

    if surface_height > 0 {
        let surface = RECT {
            left: client.left,
            top: client.top,
            right: client.right,
            bottom: client.top.saturating_add(surface_height),
        };
        // SAFETY: the HDC, rectangle, and theme-owned brush are live for paint.
        unsafe {
            FillRect(device, &raw const surface, state.theme.brushes.surface());
        }
    }
    if border_height > 0 {
        let border = RECT {
            left: client.left,
            top: client.bottom.saturating_sub(border_height),
            right: client.right,
            bottom: client.bottom,
        };
        // SAFETY: the HDC, rectangle, and theme-owned brush are live for paint.
        unsafe {
            FillRect(device, &raw const border, state.theme.brushes.border());
        }
    }

    // SAFETY: PBM_GETPOS is pointer-free and reads the live common control's
    // bounded native value, which remains the accessibility source of truth.
    let raw_position = unsafe { SendMessageW(window, PBM_GETPOS, None, None) }.0;
    let position = u64::try_from(raw_position).unwrap_or_default();
    let accent_width = fill_width(width, position, PROGRESS_MAXIMUM);
    if accent_width > 0 && height > 0 {
        let accent = RECT {
            left: client.left,
            top: client.top,
            right: client.left.saturating_add(accent_width),
            bottom: client.bottom,
        };
        // SAFETY: the HDC, rectangle, and theme-owned brush are live for paint.
        unsafe {
            FillRect(device, &raw const accent, state.theme.brushes.accent());
        }
    }
    true
}

fn parent_from_reference(reference_data: usize) -> HWND {
    HWND(reference_data as *mut c_void)
}

/// Returns the nearest-pixel accent width for a bounded progress fraction.
fn fill_width(client_width: i32, position: u64, maximum: u64) -> i32 {
    if client_width <= 0 || maximum == 0 {
        return 0;
    }
    let width = u128::from(u32::try_from(client_width).unwrap_or_default());
    let position = u128::from(position.min(maximum));
    let maximum = u128::from(maximum);
    let rounded = width.saturating_mul(position).saturating_add(maximum / 2) / maximum;
    i32::try_from(rounded).unwrap_or(client_width)
}

/// Splits the unfilled line into upper surface pixels and a bottom border.
fn background_split(client_height: i32, border_width: i32) -> (i32, i32) {
    let height = client_height.max(0);
    let border = border_width.max(0).min(height);
    (height.saturating_sub(border), border)
}

#[cfg(test)]
mod tests {
    use super::{background_split, fill_width};

    #[test]
    fn fill_width_is_rounded_clamped_and_overflow_safe() {
        assert_eq!(fill_width(1_000, 0, 100), 0);
        assert_eq!(fill_width(1_000, 1, 100), 10);
        assert_eq!(fill_width(1_000, 50, 100), 500);
        assert_eq!(fill_width(1_000, 99, 100), 990);
        assert_eq!(fill_width(1_000, 100, 100), 1_000);
        assert_eq!(fill_width(1_000, 101, 100), 1_000);
        assert_eq!(fill_width(3, 50, 100), 2);
        assert_eq!(fill_width(i32::MAX, u64::MAX, u64::MAX), i32::MAX);
    }

    #[test]
    fn fill_width_fails_closed_for_empty_geometry_or_range() {
        assert_eq!(fill_width(-1, 50, 100), 0);
        assert_eq!(fill_width(0, 50, 100), 0);
        assert_eq!(fill_width(100, 50, 0), 0);
    }

    #[test]
    fn background_split_matches_scaled_leanmark_hairline() {
        assert_eq!(background_split(2, 1), (1, 1));
        assert_eq!(background_split(3, 2), (1, 2));
        assert_eq!(background_split(4, 2), (2, 2));
        assert_eq!(background_split(1, 2), (0, 1));
        assert_eq!(background_split(0, 1), (0, 0));
        assert_eq!(background_split(-1, -1), (0, 0));
    }
}
