//! Hover tracking for the owner-drawn buttons.
//!
//! Owner-drawn buttons are not told when the mouse is over them, so each one
//! is subclassed to note it in the window state and repaint, which lets them
//! show `LeanMark`'s muted hover surface.

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{WM_MOUSEMOVE, WM_NCDESTROY};

const HOVER_SUBCLASS_ID: usize = 0x4c52_4856;

/// Tracks the mouse over `button` for the window `window`.
pub(super) fn install(button: HWND, window: HWND) -> bool {
    if button.0.is_null() || window.0.is_null() {
        return false;
    }
    // SAFETY: Both HWNDs are live UI-thread windows. The subclass keeps only
    // the numeric window HWND and removes itself during WM_NCDESTROY.
    unsafe {
        SetWindowSubclass(
            button,
            Some(hover_subclass_proc),
            HOVER_SUBCLASS_ID,
            window.0.addr(),
        )
    }
    .as_bool()
}

unsafe extern "system" fn hover_subclass_proc(
    button: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    reference_data: usize,
) -> LRESULT {
    let window = HWND(std::ptr::with_exposed_provenance_mut(reference_data));
    match message {
        WM_MOUSEMOVE => set_hover(window, button, true),
        WM_MOUSELEAVE => set_hover(window, button, false),
        WM_NCDESTROY => {
            // SAFETY: This removes the same procedure and ID installed above.
            let _ = unsafe { RemoveWindowSubclass(button, Some(hover_subclass_proc), subclass_id) };
        }
        _ => {}
    }
    // SAFETY: Every message continues to the button's own procedure.
    unsafe { DefSubclassProc(button, message, wparam, lparam) }
}

fn set_hover(window: HWND, button: HWND, inside: bool) {
    let pointer = super::state_pointer_for(window);
    // SAFETY: The state stays attached while its children handle input.
    let Some(state) = (unsafe { pointer.as_mut() }) else {
        return;
    };
    let previous = state.hover_button;
    if inside {
        if previous == button {
            return;
        }
        state.hover_button = button;
        let mut track = TRACKMOUSEEVENT {
            cbSize: u32::try_from(std::mem::size_of::<TRACKMOUSEEVENT>()).unwrap_or_default(),
            dwFlags: TME_LEAVE,
            hwndTrack: button,
            dwHoverTime: 0,
        };
        // SAFETY: The structure is writable and names a live window.
        let _ = unsafe { TrackMouseEvent(&raw mut track) };
    } else if previous == button {
        state.hover_button = HWND::default();
    } else {
        return;
    }
    for handle in [previous, button] {
        if !handle.0.is_null() {
            // SAFETY: Both are live buttons on this thread.
            let _ = unsafe { InvalidateRect(Some(handle), None, false) };
        }
    }
}
