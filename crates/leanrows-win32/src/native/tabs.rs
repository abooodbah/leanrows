//! Flat painting and close targets for the document tab strip.
//!
//! The strip stays a standard tab control, so Windows keeps exposing each tab
//! to assistive technology and keeps arrow-key selection. Only its pixels are
//! replaced to match the rest of the chrome, and each tab gets a close glyph.
//! A middle click anywhere on a tab also closes it.

use std::path::Path;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    DrawFocusRect, DrawTextW, EndPaint, FillRect, HBRUSH, HDC, HGDIOBJ, InvalidateRect,
    PAINTSTRUCT, RestoreDC, SaveDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Controls::{
    TCHITTESTINFO, TCIF_TEXT, TCITEMW, TCM_DELETEALLITEMS, TCM_GETCURSEL, TCM_GETITEMCOUNT,
    TCM_GETITEMRECT, TCM_GETITEMW, TCM_HITTEST, TCM_INSERTITEMW, TCM_SETCURSEL, TCM_SETITEMSIZE,
    WM_MOUSELEAVE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, PostMessageW, SendMessageW, WM_ERASEBKGND, WM_KILLFOCUS, WM_LBUTTONDOWN,
    WM_MBUTTONUP, WM_MOUSEMOVE, WM_NCDESTROY, WM_PAINT, WM_PRINTCLIENT, WM_SETFOCUS, WM_SETREDRAW,
};
use windows::core::PWSTR;

use super::theme::ThemeResources;
use crate::ShellError;

const TAB_SUBCLASS_ID: usize = 0x4c52_5442;
/// Longest label kept for one tab, in UTF-16 units including the terminator.
const MAX_LABEL_UNITS: usize = 260;
const MAX_LABEL_UNITS_I32: i32 = 260;
const CLOSE_GLYPH: u16 = 0x00D7;

/// The tab under the mouse, and whether the pointer is on its close glyph.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TabHover {
    pub(super) index: Option<usize>,
    pub(super) close: bool,
}

/// Installs the painter without replacing the tab control's own procedure.
pub(super) fn install_subclass(strip: HWND, parent: HWND) -> Result<(), ShellError> {
    if strip.0.is_null() || parent.0.is_null() {
        return Err(ShellError::new(
            "tab strip subclass requires live child and parent windows",
        ));
    }
    // SAFETY: Both HWNDs are live UI-thread windows. The subclass keeps only
    // the numeric parent HWND and removes itself during WM_NCDESTROY.
    let installed = unsafe {
        SetWindowSubclass(
            strip,
            Some(tab_subclass_proc),
            TAB_SUBCLASS_ID,
            parent.0.addr(),
        )
    };
    if installed.as_bool() {
        Ok(())
    } else {
        Err(ShellError::new("tab strip subclass installation failed"))
    }
}

/// Replaces every tab label and selects `selected`.
pub(super) fn set_items(strip: HWND, labels: &[String], selected: usize) {
    // SAFETY: The strip is live on this UI thread. Each TCITEMW and its text
    // buffer outlive the synchronous insert, which copies the text.
    unsafe {
        SendMessageW(strip, WM_SETREDRAW, Some(WPARAM(0)), Some(LPARAM(0)));
        SendMessageW(strip, TCM_DELETEALLITEMS, None, None);
        for (index, label) in labels.iter().enumerate() {
            let mut text: Vec<u16> = label
                .encode_utf16()
                .filter(|unit| *unit != 0)
                .take(MAX_LABEL_UNITS - 1)
                .collect();
            text.push(0);
            let item = TCITEMW {
                mask: TCIF_TEXT,
                pszText: PWSTR(text.as_mut_ptr()),
                ..Default::default()
            };
            SendMessageW(
                strip,
                TCM_INSERTITEMW,
                Some(WPARAM(index)),
                Some(LPARAM((&raw const item) as isize)),
            );
        }
    }
    select(strip, selected);
    // SAFETY: This re-enables painting for the same live control.
    unsafe { SendMessageW(strip, WM_SETREDRAW, Some(WPARAM(1)), Some(LPARAM(0))) };
    invalidate(strip);
}

/// Selects a tab without sending a selection-change notification.
pub(super) fn select(strip: HWND, index: usize) {
    // SAFETY: TCM_SETCURSEL carries an index and no pointers.
    unsafe { SendMessageW(strip, TCM_SETCURSEL, Some(WPARAM(index)), Some(LPARAM(0))) };
    invalidate(strip);
}

pub(super) fn selected_index(strip: HWND) -> Option<usize> {
    // SAFETY: TCM_GETCURSEL carries no pointers; -1 means no selection.
    let raw = unsafe { SendMessageW(strip, TCM_GETCURSEL, None, None) }.0;
    usize::try_from(raw).ok()
}

pub(super) fn item_count(strip: HWND) -> usize {
    // SAFETY: TCM_GETITEMCOUNT carries no pointers.
    let raw = unsafe { SendMessageW(strip, TCM_GETITEMCOUNT, None, None) }.0;
    usize::try_from(raw).unwrap_or_default()
}

/// Sets the fixed width and height the tab control uses for every tab.
pub(super) fn set_item_size(strip: HWND, width: i32, height: i32) {
    let width = u16::try_from(width.clamp(0, i32::from(u16::MAX))).unwrap_or(u16::MAX);
    let height = u16::try_from(height.clamp(0, i32::from(u16::MAX))).unwrap_or(u16::MAX);
    let packed = (u32::from(height) << 16) | u32::from(width);
    // SAFETY: TCM_SETITEMSIZE carries two packed 16-bit extents.
    unsafe {
        SendMessageW(
            strip,
            TCM_SETITEMSIZE,
            Some(WPARAM(0)),
            Some(LPARAM(isize::try_from(packed).unwrap_or_default())),
        );
    }
    invalidate(strip);
}

pub(super) fn invalidate(strip: HWND) {
    if strip.0.is_null() {
        return;
    }
    // SAFETY: The strip owns its update region; painting covers the client.
    let _ = unsafe { InvalidateRect(Some(strip), None, false) };
}

/// Tab labels for a set of open files: the file name, plus the parent folder
/// when two open files share a name.
pub(super) fn labels(paths: &[Option<&Path>]) -> Vec<String> {
    let names: Vec<String> = paths
        .iter()
        .map(|path| {
            path.and_then(Path::file_name).map_or_else(
                || String::from("LeanRows"),
                |name| name.to_string_lossy().into_owned(),
            )
        })
        .collect();
    let folded: Vec<String> = names.iter().map(|name| name.to_lowercase()).collect();
    names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let shared = folded
                .iter()
                .enumerate()
                .any(|(other, candidate)| other != index && *candidate == folded[index]);
            let parent = paths[index]
                .and_then(Path::parent)
                .and_then(Path::file_name);
            match parent {
                Some(parent) if shared => format!("{name} ({})", parent.to_string_lossy()),
                _ => name.clone(),
            }
        })
        .collect()
}

unsafe extern "system" fn tab_subclass_proc(
    strip: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    reference_data: usize,
) -> LRESULT {
    let parent = HWND(std::ptr::with_exposed_provenance_mut(reference_data));
    match message {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT if paint_window(strip, parent) => LRESULT(0),
        WM_PRINTCLIENT => {
            let device = HDC(std::ptr::with_exposed_provenance_mut(wparam.0));
            if !device.0.is_null() && paint_client(strip, parent, device) {
                LRESULT(0)
            } else {
                // SAFETY: Unpaintable requests stay with the original control.
                unsafe { DefSubclassProc(strip, message, wparam, lparam) }
            }
        }
        WM_MOUSEMOVE => {
            track_hover(strip, parent, point_from_lparam(lparam));
            // SAFETY: The native control keeps its own mouse handling.
            unsafe { DefSubclassProc(strip, message, wparam, lparam) }
        }
        WM_MOUSELEAVE => {
            set_hover(strip, parent, TabHover::default());
            // SAFETY: The native control keeps its own mouse handling.
            unsafe { DefSubclassProc(strip, message, wparam, lparam) }
        }
        WM_LBUTTONDOWN => {
            let hover = hit_test(strip, parent, point_from_lparam(lparam));
            if hover.close
                && let Some(index) = hover.index
            {
                request_close(parent, index);
                LRESULT(0)
            } else {
                // SAFETY: Clicks outside a close glyph select the tab natively.
                unsafe { DefSubclassProc(strip, message, wparam, lparam) }
            }
        }
        WM_MBUTTONUP => {
            if let Some(index) = hit_test(strip, parent, point_from_lparam(lparam)).index {
                request_close(parent, index);
            }
            LRESULT(0)
        }
        WM_SETFOCUS | WM_KILLFOCUS => {
            invalidate(strip);
            // SAFETY: Focus changes continue through the native control.
            unsafe { DefSubclassProc(strip, message, wparam, lparam) }
        }
        WM_NCDESTROY => {
            // SAFETY: This removes the exact procedure/id pair being invoked;
            // the original procedure must still receive WM_NCDESTROY.
            let _ = unsafe { RemoveWindowSubclass(strip, Some(tab_subclass_proc), subclass_id) };
            // SAFETY: Delegation preserves the control's own teardown.
            unsafe { DefSubclassProc(strip, message, wparam, lparam) }
        }
        _ => {
            // SAFETY: Keyboard selection, hit testing, and accessibility stay
            // with the native tab control.
            unsafe { DefSubclassProc(strip, message, wparam, lparam) }
        }
    }
}

fn paint_window(strip: HWND, parent: HWND) -> bool {
    let state_pointer = super::state_pointer_for(parent);
    // SAFETY: The subclass runs on the owning UI thread, and WindowState stays
    // attached to the parent until the parent's own WM_NCDESTROY.
    let Some(state) = (unsafe { state_pointer.as_ref() }) else {
        return false;
    };
    let mut paint = PAINTSTRUCT::default();
    // SAFETY: The strip is handling WM_PAINT and paint is writable.
    let device = unsafe { BeginPaint(strip, &raw mut paint) };
    if !device.0.is_null() {
        paint_strip(strip, device, &state.theme, state.tab_hover);
    }
    // SAFETY: This closes the BeginPaint call above.
    let _ = unsafe { EndPaint(strip, &raw const paint) };
    true
}

fn paint_client(strip: HWND, parent: HWND, device: HDC) -> bool {
    let state_pointer = super::state_pointer_for(parent);
    // SAFETY: WM_PRINTCLIENT is synchronous on the owning UI thread.
    let Some(state) = (unsafe { state_pointer.as_ref() }) else {
        return false;
    };
    paint_strip(strip, device, &state.theme, state.tab_hover);
    true
}

fn paint_strip(strip: HWND, device: HDC, theme: &ThemeResources, hover: TabHover) {
    let mut client = RECT::default();
    // SAFETY: The strip is live and client is writable.
    if unsafe { GetClientRect(strip, &raw mut client) }.is_err() {
        return;
    }
    let border = theme.metrics.border_width.max(1);
    let rule = RECT {
        top: client.bottom.saturating_sub(border).max(client.top),
        ..client
    };
    // SAFETY: Theme brushes outlive the strip and this synchronous paint.
    unsafe {
        FillRect(device, &raw const client, theme.brushes.surface());
        FillRect(device, &raw const rule, theme.brushes.border());
    }
    // SaveDC makes restoring the font and colors a single step.
    // SAFETY: device comes from BeginPaint or WM_PRINTCLIENT.
    let saved = unsafe { SaveDC(device) };
    if saved == 0 {
        return;
    }
    // SAFETY: device is live for this paint.
    unsafe { SetBkMode(device, TRANSPARENT) };
    let selected = selected_index(strip);
    // SAFETY: GetFocus reads this thread's focus window.
    let focused = unsafe { GetFocus() } == strip;
    for index in 0..item_count(strip) {
        let Some(bounds) = tab_bounds(strip, index, client, border) else {
            continue;
        };
        let is_selected = selected == Some(index);
        paint_tab(
            strip,
            device,
            theme,
            TabPaint {
                index,
                bounds,
                client_bottom: client.bottom,
                selected: is_selected,
                hovered: hover.index == Some(index),
                close_hovered: hover.index == Some(index) && hover.close,
                focused: focused && is_selected,
            },
        );
    }
    // SAFETY: saved came from SaveDC on the same device.
    let _ = unsafe { RestoreDC(device, saved) };
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "each flag is one independent visual state of a tab"
)]
#[derive(Clone, Copy)]
struct TabPaint {
    index: usize,
    bounds: RECT,
    client_bottom: i32,
    selected: bool,
    hovered: bool,
    close_hovered: bool,
    focused: bool,
}

fn paint_tab(strip: HWND, device: HDC, theme: &ThemeResources, tab: TabPaint) {
    let metrics = theme.metrics;
    let palette = theme.palette;
    let bounds = tab.bounds;
    if tab.hovered && !tab.selected {
        fill(device, bounds, theme.brushes.surface_muted());
    }
    let close = close_rect(bounds, metrics.icon_command, metrics.space_2);
    let mut text = [0_u16; MAX_LABEL_UNITS];
    let length = item_text(strip, tab.index, &mut text);
    let mut label = RECT {
        left: bounds.left.saturating_add(metrics.space_3),
        right: close.left.saturating_sub(metrics.space_1),
        ..bounds
    };
    let (font, color) = if tab.selected {
        (theme.fonts.semibold(), palette.text_primary)
    } else {
        (theme.fonts.body(), palette.text_secondary)
    };
    // SAFETY: The theme owns the fonts, and the text slice and rectangles are
    // local storage that outlives each synchronous call.
    unsafe {
        SelectObject(device, HGDIOBJ(font.0));
        SetTextColor(device, color.colorref());
        if label.right > label.left && length > 0 {
            DrawTextW(
                device,
                &mut text[..length],
                &raw mut label,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
        }
    }
    if tab.close_hovered {
        let brush = if tab.selected {
            theme.brushes.surface_muted()
        } else {
            theme.brushes.border()
        };
        fill(device, close, brush);
    }
    let glyph_color = if tab.close_hovered {
        palette.text_primary
    } else {
        palette.text_secondary
    };
    let mut glyph = [CLOSE_GLYPH];
    let mut glyph_bounds = close;
    // SAFETY: As above; the glyph buffer is local.
    unsafe {
        SelectObject(device, HGDIOBJ(theme.fonts.body().0));
        SetTextColor(device, glyph_color.colorref());
        DrawTextW(
            device,
            &mut glyph,
            &raw mut glyph_bounds,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
    }
    let border = metrics.border_width.max(1);
    if tab.selected {
        let underline = RECT {
            left: bounds.left,
            top: tab.client_bottom.saturating_sub(metrics.focus_width.max(1)),
            right: bounds.right,
            bottom: tab.client_bottom,
        };
        fill(device, underline, theme.brushes.accent());
    } else {
        let separator = RECT {
            left: bounds.right.saturating_sub(border),
            top: bounds.top.saturating_add(metrics.space_2),
            right: bounds.right,
            bottom: bounds.bottom.saturating_sub(metrics.space_2),
        };
        if separator.bottom > separator.top {
            fill(device, separator, theme.brushes.border());
        }
    }
    if tab.focused {
        let inset = metrics.space_1.max(2);
        let focus = RECT {
            left: bounds.left.saturating_add(inset),
            top: bounds.top.saturating_add(inset),
            right: bounds.right.saturating_sub(inset),
            bottom: bounds.bottom.saturating_sub(inset),
        };
        // SAFETY: The rectangle is local and the device is live.
        let _ = unsafe { DrawFocusRect(device, &raw const focus) };
    }
}

fn fill(device: HDC, bounds: RECT, brush: HBRUSH) {
    // SAFETY: The theme owns the brush and the rectangle is local.
    unsafe { FillRect(device, &raw const bounds, brush) };
}

fn item_text(strip: HWND, index: usize, text: &mut [u16; MAX_LABEL_UNITS]) -> usize {
    let mut item = TCITEMW {
        mask: TCIF_TEXT,
        pszText: PWSTR(text.as_mut_ptr()),
        cchTextMax: MAX_LABEL_UNITS_I32,
        ..Default::default()
    };
    // SAFETY: item and its fixed buffer stay writable for the synchronous
    // call, and cchTextMax matches the buffer length.
    let found = unsafe {
        SendMessageW(
            strip,
            TCM_GETITEMW,
            Some(WPARAM(index)),
            Some(LPARAM((&raw mut item) as isize)),
        )
    }
    .0 != 0;
    if !found {
        return 0;
    }
    text.iter()
        .position(|unit| *unit == 0)
        .unwrap_or(text.len())
}

/// The rectangle the tab control assigns to one tab.
pub(super) fn item_rect(strip: HWND, index: usize) -> Option<RECT> {
    let mut item = RECT::default();
    // SAFETY: item is writable and the synchronous message keeps no pointer.
    let found = unsafe {
        SendMessageW(
            strip,
            TCM_GETITEMRECT,
            Some(WPARAM(index)),
            Some(LPARAM((&raw mut item) as isize)),
        )
    }
    .0 != 0;
    found.then_some(item)
}

/// The full-height rectangle of one tab above the strip's bottom rule.
fn tab_bounds(strip: HWND, index: usize, client: RECT, border: i32) -> Option<RECT> {
    let item = item_rect(strip, index)?;
    let bounds = RECT {
        left: item.left,
        top: client.top,
        right: item.right,
        bottom: client.bottom.saturating_sub(border),
    };
    (bounds.right > bounds.left && bounds.bottom > bounds.top).then_some(bounds)
}

/// The square target for a tab's close glyph, inset from its right edge.
fn close_rect(tab: RECT, size: i32, inset: i32) -> RECT {
    let right = tab.right.saturating_sub(inset).max(tab.left);
    let left = right.saturating_sub(size).max(tab.left);
    let height = tab.bottom.saturating_sub(tab.top);
    let top = tab
        .top
        .saturating_add(height.saturating_sub(size).max(0) / 2);
    RECT {
        left,
        top,
        right,
        bottom: top.saturating_add(size).min(tab.bottom),
    }
}

fn contains(bounds: RECT, point: POINT) -> bool {
    point.x >= bounds.left
        && point.x < bounds.right
        && point.y >= bounds.top
        && point.y < bounds.bottom
}

fn hit_test(strip: HWND, parent: HWND, point: POINT) -> TabHover {
    let mut info = TCHITTESTINFO {
        pt: point,
        ..Default::default()
    };
    // SAFETY: info is writable for this synchronous hit test.
    let raw = unsafe {
        SendMessageW(
            strip,
            TCM_HITTEST,
            None,
            Some(LPARAM((&raw mut info) as isize)),
        )
    }
    .0;
    let Ok(index) = usize::try_from(raw) else {
        return TabHover::default();
    };
    let state_pointer = super::state_pointer_for(parent);
    // SAFETY: The parent state stays attached while its children handle input.
    let Some(state) = (unsafe { state_pointer.as_ref() }) else {
        return TabHover::default();
    };
    let mut client = RECT::default();
    // SAFETY: The strip is live and client is writable.
    if unsafe { GetClientRect(strip, &raw mut client) }.is_err() {
        return TabHover::default();
    }
    let metrics = state.theme.metrics;
    let close =
        tab_bounds(strip, index, client, metrics.border_width.max(1)).is_some_and(|bounds| {
            contains(
                close_rect(bounds, metrics.icon_command, metrics.space_2),
                point,
            )
        });
    TabHover {
        index: Some(index),
        close,
    }
}

fn track_hover(strip: HWND, parent: HWND, point: POINT) {
    let hover = hit_test(strip, parent, point);
    let state_pointer = super::state_pointer_for(parent);
    // SAFETY: Mouse input is dispatched from the message loop, where no other
    // borrow of the window state is live.
    let Some(state) = (unsafe { state_pointer.as_mut() }) else {
        return;
    };
    if hover.index.is_some() && state.tab_hover.index.is_none() {
        let mut request = TRACKMOUSEEVENT {
            cbSize: u32::try_from(size_of::<TRACKMOUSEEVENT>()).unwrap_or_default(),
            dwFlags: TME_LEAVE,
            hwndTrack: strip,
            dwHoverTime: 0,
        };
        // SAFETY: The request is initialized for this live window.
        let _ = unsafe { TrackMouseEvent(&raw mut request) };
    }
    set_hover(strip, parent, hover);
}

fn set_hover(strip: HWND, parent: HWND, hover: TabHover) {
    let state_pointer = super::state_pointer_for(parent);
    // SAFETY: As in `track_hover`.
    let Some(state) = (unsafe { state_pointer.as_mut() }) else {
        return;
    };
    if state.tab_hover != hover {
        state.tab_hover = hover;
        invalidate(strip);
    }
}

fn request_close(parent: HWND, index: usize) {
    let state_pointer = super::state_pointer_for(parent);
    // SAFETY: The parent state stays attached while its children handle input.
    let Some(id) =
        (unsafe { state_pointer.as_ref() }).and_then(|state| state.tab_order.get(index).copied())
    else {
        return;
    };
    // The close runs from the parent's queue, after this control has
    // finished handling the click that asked for it.
    // SAFETY: The message carries a tab id and no pointers.
    let _ = unsafe { PostMessageW(Some(parent), super::WM_APP_CLOSE_TAB, WPARAM(id), LPARAM(0)) };
}

fn point_from_lparam(lparam: LPARAM) -> POINT {
    let low = u16::try_from(lparam.0 & 0xffff).unwrap_or_default();
    let high = u16::try_from((lparam.0 >> 16) & 0xffff).unwrap_or_default();
    POINT {
        x: i32::from(i16::from_ne_bytes(low.to_ne_bytes())),
        y: i32::from(i16::from_ne_bytes(high.to_ne_bytes())),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use windows::Win32::Foundation::{LPARAM, POINT, RECT};

    use super::{close_rect, contains, labels, point_from_lparam};

    #[test]
    fn labels_add_the_folder_only_when_file_names_collide() {
        let first = Path::new(r"C:\logs\2026\app.log");
        let second = Path::new(r"D:\archive\APP.LOG");
        let third = Path::new(r"C:\data\rows.csv");
        assert_eq!(
            labels(&[Some(first), Some(second), Some(third), None]),
            vec![
                String::from("app.log (2026)"),
                String::from("APP.LOG (archive)"),
                String::from("rows.csv"),
                String::from("LeanRows"),
            ]
        );
    }

    #[test]
    fn close_target_is_a_centered_square_inside_the_tab() {
        let tab = RECT {
            left: 100,
            top: 0,
            right: 300,
            bottom: 35,
        };
        let close = close_rect(tab, 20, 8);
        assert_eq!(
            close,
            RECT {
                left: 272,
                top: 7,
                right: 292,
                bottom: 27,
            }
        );
        assert!(contains(close, POINT { x: 272, y: 7 }));
        assert!(!contains(close, POINT { x: 292, y: 7 }));
        let narrow = RECT {
            left: 0,
            top: 0,
            right: 10,
            bottom: 8,
        };
        let squeezed = close_rect(narrow, 20, 8);
        assert!(squeezed.left >= narrow.left && squeezed.right <= narrow.right);
        assert!(squeezed.top >= narrow.top && squeezed.bottom <= narrow.bottom);
    }

    #[test]
    fn mouse_coordinates_keep_their_sign() {
        assert_eq!(
            point_from_lparam(LPARAM(0x0020_0010)),
            POINT { x: 16, y: 32 }
        );
        assert_eq!(
            point_from_lparam(LPARAM(0xFFFE_FFFF)),
            POINT { x: -1, y: -2 }
        );
    }
}
