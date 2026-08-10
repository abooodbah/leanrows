//! LeanMark-aligned painting for the native list-view header.
//!
//! The header remains a standard common control. Only its client painting is
//! replaced, so documented header hit testing, column tracking, and resizing
//! continue to be handled by the original window procedure.

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE,
    DT_VCENTER, DrawTextW, FillRect, HDC, HGDIOBJ, PAINTSTRUCT, RestoreDC, SaveDC, SelectObject,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Controls::{
    HDF_CENTER, HDF_JUSTIFYMASK, HDF_RIGHT, HDI_FORMAT, HDI_TEXT, HDITEMW, HDM_GETITEMCOUNT,
    HDM_GETITEMRECT, HDM_GETITEMW, HEADER_CONTROL_FORMAT_FLAGS,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, SendMessageW, WM_ERASEBKGND, WM_NCDESTROY, WM_PAINT, WM_PRINTCLIENT,
};
use windows::core::PWSTR;

use super::theme::ThemeResources;
use crate::ShellError;

const HEADER_SUBCLASS_ID: usize = 0x4c52_4844;
const MAX_HEADER_ITEMS: usize = 256;
const MAX_HEADER_TEXT_UNITS: usize = 256;
const MAX_HEADER_TEXT_UNITS_I32: i32 = 256;

/// Installs the header painter without replacing the common control's window
/// procedure. Reinstalling the same `(procedure, id)` pair updates its parent
/// reference data rather than stacking another subclass.
pub(super) fn install_subclass(
    header_hwnd: HWND,
    parent_window_hwnd: HWND,
) -> Result<(), ShellError> {
    if header_hwnd.0.is_null() || parent_window_hwnd.0.is_null() {
        return Err(ShellError::new(
            "native list header subclass received an invalid window",
        ));
    }

    // SAFETY: Both HWNDs are live UI-thread windows. The subclass retains only
    // the numeric parent HWND, not a Rust reference, and removes itself during
    // the header's WM_NCDESTROY processing.
    let installed = unsafe {
        SetWindowSubclass(
            header_hwnd,
            Some(header_subclass_proc),
            HEADER_SUBCLASS_ID,
            parent_window_hwnd.0.addr(),
        )
    };
    if installed.as_bool() {
        Ok(())
    } else {
        Err(ShellError::new(
            "native list header subclass installation failed",
        ))
    }
}

unsafe extern "system" fn header_subclass_proc(
    header: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    reference_data: usize,
) -> LRESULT {
    let parent = HWND(std::ptr::with_exposed_provenance_mut::<c_void>(
        reference_data,
    ));
    match message {
        WM_PAINT if paint_window(header, parent) => LRESULT(0),
        WM_PRINTCLIENT => {
            let device = HDC(std::ptr::with_exposed_provenance_mut::<c_void>(wparam.0));
            if !device.0.is_null() && paint_client(header, parent, device) {
                LRESULT(0)
            } else {
                // SAFETY: Unhandled or unpaintable messages remain owned by the
                // original common-control procedure.
                unsafe { DefSubclassProc(header, message, wparam, lparam) }
            }
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_NCDESTROY => {
            // SAFETY: This removes the exact procedure/id pair currently being
            // invoked. The original procedure must still receive WM_NCDESTROY.
            let _ =
                unsafe { RemoveWindowSubclass(header, Some(header_subclass_proc), subclass_id) };
            // SAFETY: Delegation preserves the header control's destruction.
            unsafe { DefSubclassProc(header, message, wparam, lparam) }
        }
        _ => {
            // SAFETY: Every non-paint interaction stays with the native header,
            // including divider hit testing, tracking, and keyboard behavior.
            unsafe { DefSubclassProc(header, message, wparam, lparam) }
        }
    }
}

fn paint_window(header: HWND, parent: HWND) -> bool {
    let state_pointer = super::state_pointer_for(parent);
    // SAFETY: The subclass runs on the owning UI thread. WindowState remains
    // attached to the parent until its own WM_NCDESTROY processing.
    let Some(state) = (unsafe { state_pointer.as_ref() }) else {
        return false;
    };

    let mut paint = PAINTSTRUCT::default();
    // SAFETY: The header is handling WM_PAINT and paint is writable for the
    // duration of the matching BeginPaint/EndPaint pair.
    let device = unsafe { BeginPaint(header, &raw mut paint) };
    if !device.0.is_null() {
        paint_header(header, device, &state.theme);
    }
    // SAFETY: This exactly closes the successful or empty BeginPaint call.
    let _ = unsafe { windows::Win32::Graphics::Gdi::EndPaint(header, &raw const paint) };
    true
}

fn paint_client(header: HWND, parent: HWND, device: HDC) -> bool {
    let state_pointer = super::state_pointer_for(parent);
    // SAFETY: WM_PRINTCLIENT is synchronous on the owning UI thread and the
    // parent-owned state outlives this call.
    let Some(state) = (unsafe { state_pointer.as_ref() }) else {
        return false;
    };
    paint_header(header, device, &state.theme);
    true
}

fn paint_header(header: HWND, device: HDC, theme: &ThemeResources) {
    let mut client = RECT::default();
    // SAFETY: The header is live and client is writable.
    if unsafe { GetClientRect(header, &raw mut client) }.is_err() {
        return;
    }

    // SAFETY: Theme brushes outlive the header and this synchronous paint.
    unsafe { FillRect(device, &raw const client, theme.brushes.surface_muted()) };

    let border_width = theme.metrics.border_width.max(1);
    if let Some(border) = bottom_border_rect(client, border_width) {
        // SAFETY: The bounded rectangle and brush are valid for this HDC.
        unsafe { FillRect(device, &raw const border, theme.brushes.border()) };
    }

    // SaveDC makes all text-related state restoration atomic. If the device
    // cannot save state, the background and structural border remain usable.
    // SAFETY: device is supplied by BeginPaint or WM_PRINTCLIENT.
    let saved_device = unsafe { SaveDC(device) };
    if saved_device == 0 {
        return;
    }
    // SAFETY: The selected font and colors are owned by the current theme and
    // remain valid until RestoreDC below.
    unsafe {
        SelectObject(device, HGDIOBJ(theme.fonts.semibold().0));
        SetBkMode(device, TRANSPARENT);
        SetTextColor(device, theme.palette.text_secondary.colorref());
    }

    // SAFETY: The message has no pointer payload and synchronously returns the
    // current header item count.
    let raw_count = unsafe { SendMessageW(header, HDM_GETITEMCOUNT, None, None) }.0;
    for item_index in 0..bounded_item_count(raw_count) {
        paint_item(header, device, theme, item_index, border_width);
    }

    // SAFETY: saved_device was returned by SaveDC for this same HDC.
    let _ = unsafe { RestoreDC(device, saved_device) };
}

fn paint_item(
    header: HWND,
    device: HDC,
    theme: &ThemeResources,
    item_index: usize,
    border_width: i32,
) {
    let mut item_bounds = RECT::default();
    // SAFETY: item_bounds is writable and the synchronous header message does
    // not retain its pointer.
    let has_bounds = unsafe {
        SendMessageW(
            header,
            HDM_GETITEMRECT,
            Some(WPARAM(item_index)),
            Some(LPARAM((&raw mut item_bounds) as isize)),
        )
    }
    .0 != 0;
    if !has_bounds {
        return;
    }

    if let Some(separator) = right_border_rect(item_bounds, border_width) {
        // SAFETY: The bounded rectangle and theme brush remain valid for this
        // synchronous fill.
        unsafe { FillRect(device, &raw const separator, theme.brushes.border()) };
    }

    let Some(mut text_bounds) = text_rect(item_bounds, theme.metrics.space_3, border_width) else {
        return;
    };
    let mut text = [0_u16; MAX_HEADER_TEXT_UNITS];
    let mut item = HDITEMW {
        mask: HDI_TEXT | HDI_FORMAT,
        pszText: PWSTR(text.as_mut_ptr()),
        cchTextMax: MAX_HEADER_TEXT_UNITS_I32,
        ..Default::default()
    };
    // SAFETY: item and its fixed text buffer remain writable for the complete
    // synchronous HDM_GETITEMW call. cchTextMax matches the allocation bound.
    let has_item = unsafe {
        SendMessageW(
            header,
            HDM_GETITEMW,
            Some(WPARAM(item_index)),
            Some(LPARAM((&raw mut item) as isize)),
        )
    }
    .0 != 0;
    if !has_item {
        return;
    }

    let text_length = bounded_utf16_length(&text);
    if text_length == 0 {
        return;
    }
    let format = draw_text_format(item.fmt);
    // SAFETY: The mutable slice is bounded to initialized stack storage and the
    // target rectangle lives through this synchronous call.
    let _ = unsafe {
        DrawTextW(
            device,
            &mut text[..text_length],
            &raw mut text_bounds,
            format,
        )
    };
}

fn bounded_item_count(raw_count: isize) -> usize {
    usize::try_from(raw_count)
        .unwrap_or_default()
        .min(MAX_HEADER_ITEMS)
}

fn bounded_utf16_length(text: &[u16]) -> usize {
    text.iter()
        .position(|unit| *unit == 0)
        .unwrap_or(text.len())
}

fn text_rect(item: RECT, horizontal_padding: i32, border_width: i32) -> Option<RECT> {
    let padding = horizontal_padding.max(0);
    let border = border_width.max(0);
    let left = item.left.saturating_add(padding);
    let right = item.right.saturating_sub(padding.saturating_add(border));
    let bottom = item.bottom.saturating_sub(border);
    (right > left && bottom > item.top).then_some(RECT {
        left,
        top: item.top,
        right,
        bottom,
    })
}

fn bottom_border_rect(client: RECT, border_width: i32) -> Option<RECT> {
    let width = border_width.max(1);
    let top = client.bottom.saturating_sub(width).max(client.top);
    (client.right > client.left && client.bottom > client.top).then_some(RECT {
        left: client.left,
        top,
        right: client.right,
        bottom: client.bottom,
    })
}

fn right_border_rect(item: RECT, border_width: i32) -> Option<RECT> {
    let width = border_width.max(1);
    let left = item.right.saturating_sub(width).max(item.left);
    (item.right > item.left && item.bottom > item.top).then_some(RECT {
        left,
        top: item.top,
        right: item.right,
        bottom: item.bottom,
    })
}

fn draw_text_format(
    header_format: HEADER_CONTROL_FORMAT_FLAGS,
) -> windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT {
    let alignment = match header_format.0 & HDF_JUSTIFYMASK.0 {
        value if value == HDF_RIGHT.0 => DT_RIGHT,
        value if value == HDF_CENTER.0 => DT_CENTER,
        _ => DT_LEFT,
    };
    alignment | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_HEADER_ITEMS, MAX_HEADER_TEXT_UNITS, bottom_border_rect, bounded_item_count,
        bounded_utf16_length, draw_text_format, right_border_rect, text_rect,
    };
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{DT_CENTER, DT_END_ELLIPSIS, DT_RIGHT};
    use windows::Win32::UI::Controls::{HDF_CENTER, HDF_LEFT, HDF_RIGHT};

    #[test]
    fn item_count_is_nonnegative_and_bounded() {
        assert_eq!(bounded_item_count(-1), 0);
        assert_eq!(bounded_item_count(65), 65);
        assert_eq!(bounded_item_count(isize::MAX), MAX_HEADER_ITEMS);
    }

    #[test]
    fn utf16_scan_never_crosses_the_fixed_buffer() {
        let mut text = [u16::from(b'x'); MAX_HEADER_TEXT_UNITS];
        text[7] = 0;
        assert_eq!(bounded_utf16_length(&text), 7);
        text[7] = u16::from(b'x');
        assert_eq!(bounded_utf16_length(&text), MAX_HEADER_TEXT_UNITS);
    }

    #[test]
    fn text_geometry_reserves_padding_and_structural_borders() {
        let item = RECT {
            left: 10,
            top: 2,
            right: 110,
            bottom: 34,
        };
        assert_eq!(
            text_rect(item, 12, 1),
            Some(RECT {
                left: 22,
                top: 2,
                right: 97,
                bottom: 33,
            })
        );
        assert_eq!(text_rect(item, 80, 1), None);
    }

    #[test]
    fn borders_stay_inside_their_source_rectangles() {
        let bounds = RECT {
            left: 0,
            top: 0,
            right: 120,
            bottom: 32,
        };
        assert_eq!(
            bottom_border_rect(bounds, 2),
            Some(RECT {
                left: 0,
                top: 30,
                right: 120,
                bottom: 32,
            })
        );
        assert_eq!(
            right_border_rect(bounds, 2),
            Some(RECT {
                left: 118,
                top: 0,
                right: 120,
                bottom: 32,
            })
        );
    }

    #[test]
    fn native_header_alignment_is_preserved_with_ellipsis() {
        assert_eq!(draw_text_format(HDF_RIGHT).0 & DT_RIGHT.0, DT_RIGHT.0);
        assert_eq!(draw_text_format(HDF_CENTER).0 & DT_CENTER.0, DT_CENTER.0);
        assert_ne!(draw_text_format(HDF_LEFT).0 & DT_END_ELLIPSIS.0, 0);
    }
}
