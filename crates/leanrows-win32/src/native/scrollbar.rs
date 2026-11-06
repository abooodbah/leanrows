//! Thin, theme-coloured scroll bars for the list views.
//!
//! Each list keeps its own scroll bars and its own scroll state, so the
//! wheel, the keyboard, touch panning and UI Automation work as before. The
//! list's subclass narrows the room Windows keeps for those bars to a thin
//! strip, which pushes the system bars out of sight past the list's edge.
//! Two small windows laid over the strip draw the bars in the theme's
//! colours, without arrow buttons, and turn mouse input into the list's own
//! scroll messages. High contrast turns the overlay off, which gives the
//! system bars and their system colours back.

use std::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, DC_BRUSH, DC_PEN, EndPaint, FillRect, GetStockObject, InvalidateRect,
    MapWindowPoints, PAINTSTRUCT, RoundRect, SelectObject, SetDCBrushColor, SetDCPenColor,
};
use windows::Win32::UI::Controls::{
    LVIR_BOUNDS, LVM_GETHEADER, LVM_GETITEMRECT, LVM_GETTOPINDEX, LVM_SCROLL, WM_MOUSELEAVE,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GWL_STYLE, GWLP_ID, GWLP_USERDATA,
    GetClientRect, GetParent, GetScrollInfo, GetWindowLongPtrW, GetWindowRect, HMENU, HWND_TOP,
    IDC_ARROW, IsWindow, IsWindowVisible, KillTimer, LoadCursorW, MA_NOACTIVATE, RegisterClassExW,
    SB_ENDSCROLL, SB_HORZ, SB_PAGEDOWN, SB_PAGEUP, SB_VERT, SCROLLINFO, SIF_ALL, SM_CXVSCROLL,
    SM_CYHSCROLL, SWP_FRAMECHANGED, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SWP_SHOWWINDOW, SendMessageW, SetTimer, SetWindowLongPtrW, SetWindowPos,
    WM_CAPTURECHANGED, WM_ERASEBKGND, WM_HSCROLL, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCALCSIZE,
    WM_NCDESTROY, WM_PAINT, WM_SETREDRAW, WM_SIZE, WM_TIMER, WM_VSCROLL, WM_WINDOWPOSCHANGED,
    WNDCLASSEXW, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_NOACTIVATE, WS_HSCROLL, WS_VSCROLL,
};
use windows::core::{PCWSTR, w};

use super::theme::ThemeResources;
use crate::ShellError;
use crate::scroll::{ScrollRange, Thumb, page_direction, position_at, thumb};

const SUBCLASS_ID: usize = 0x4c52_5342;
const CLASS_NAME: PCWSTR = w!("LeanRowsScrollBar");
const VERTICAL_ID: isize = 1;
const HORIZONTAL_ID: isize = 2;
const REPEAT_TIMER: usize = 1;
const REPEAT_FIRST_MS: u32 = 400;
const REPEAT_MS: u32 = 50;

/// The strip a bar takes along the list's edge, in device-independent pixels.
pub(super) const STRIP_DIP: i32 = 10;
const THUMB_DIP: i32 = 6;
const THUMB_HOT_DIP: i32 = 8;
const MIN_THUMB_DIP: i32 = 32;
const END_INSET_DIP: i32 = 2;

/// Pixels for `dip` device-independent pixels at `dpi`.
#[must_use]
pub(super) fn pixels(dip: i32, dpi: u32) -> i32 {
    let dpi = i64::from(dpi.max(96));
    i32::try_from((i64::from(dip) * dpi + 48) / 96).unwrap_or(dip)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Axis {
    Vertical,
    Horizontal,
}

#[derive(Debug)]
struct Bar {
    window: HWND,
    axis: Axis,
    range: ScrollRange,
    /// Where the bar sits in the list parent's client coordinates.
    place: RECT,
    shown: bool,
    /// The part of a vertical bar beside the column titles, which is not track.
    lead: i32,
    hot: bool,
    /// While the thumb is dragged: how far into the thumb it was caught.
    grab: Option<i32>,
    /// While the track is held: the direction it pages, and where.
    paging: i32,
    held_at: i32,
}

impl Bar {
    fn new(window: HWND, axis: Axis) -> Self {
        Self {
            window,
            axis,
            range: ScrollRange::default(),
            place: RECT::default(),
            shown: false,
            lead: 0,
            hot: false,
            grab: None,
            paging: 0,
            held_at: 0,
        }
    }
}

/// One list's overlay: the list, the window whose state holds the theme, and
/// the two bars.
#[derive(Debug)]
struct Overlay {
    list: HWND,
    owner: HWND,
    enabled: bool,
    syncing: bool,
    bars: [Bar; 2],
}

/// Registers the bar window class once per process.
pub(super) fn register_class(
    instance: windows::Win32::Foundation::HINSTANCE,
) -> Result<(), ShellError> {
    let class = WNDCLASSEXW {
        cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap_or_default(),
        lpfnWndProc: Some(bar_proc),
        hInstance: instance,
        // SAFETY: IDC_ARROW is a predefined system cursor.
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    // SAFETY: The class structure and its static name outlive the call.
    if unsafe { RegisterClassExW(&raw const class) } == 0 {
        return Err(ShellError::new("scroll bar class registration failed"));
    }
    Ok(())
}

/// Lays the thin bars over `list`. `owner` is the top-level window whose
/// state holds the theme. The overlay is on unless the theme is high
/// contrast.
pub(super) fn install(list: HWND, owner: HWND, enabled: bool) -> Result<(), ShellError> {
    // SAFETY: The list is a live child on this UI thread.
    let parent = unsafe { GetParent(list) }
        .map_err(|_| ShellError::new("the list has no parent for its scroll bars"))?;
    // SAFETY: As above; the module handle is only read.
    let instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
        .map_err(|_| ShellError::new("scroll bars need the module handle"))?;
    let create = |id: isize| {
        // SAFETY: The class is registered; the parent and module belong to
        // this thread. The bar starts hidden and is placed by `sync`.
        unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE,
                CLASS_NAME,
                w!(""),
                WS_CHILD | WS_CLIPSIBLINGS,
                0,
                0,
                0,
                0,
                Some(parent),
                Some(HMENU(std::ptr::without_provenance_mut::<c_void>(
                    id.unsigned_abs(),
                ))),
                Some(windows::Win32::Foundation::HINSTANCE(instance.0)),
                None,
            )
        }
        .map_err(|_| ShellError::new("a scroll bar window could not be created"))
    };
    let vertical = create(VERTICAL_ID)?;
    let horizontal = match create(HORIZONTAL_ID) {
        Ok(horizontal) => horizontal,
        Err(error) => {
            // SAFETY: The bar was created above and holds no state yet.
            let _ = unsafe { DestroyWindow(vertical) };
            return Err(error);
        }
    };
    let overlay = Box::into_raw(Box::new(Overlay {
        list,
        owner,
        enabled,
        syncing: false,
        bars: [
            Bar::new(vertical, Axis::Vertical),
            Bar::new(horizontal, Axis::Horizontal),
        ],
    }));
    // SAFETY: The overlay outlives both bars and the list's subclass: the
    // list's WM_NCDESTROY destroys the bars, then frees it. Each bar clears
    // its own handle from it if it goes first.
    unsafe {
        SetWindowLongPtrW(vertical, GWLP_USERDATA, overlay as isize);
        SetWindowLongPtrW(horizontal, GWLP_USERDATA, overlay as isize);
        let style = GetWindowLongPtrW(list, GWL_STYLE);
        SetWindowLongPtrW(list, GWL_STYLE, style | style_bit(WS_CLIPSIBLINGS));
        if !SetWindowSubclass(list, Some(list_proc), SUBCLASS_ID, overlay as usize).as_bool() {
            let _ = DestroyWindow(vertical);
            let _ = DestroyWindow(horizontal);
            drop(Box::from_raw(overlay));
            return Err(ShellError::new("the list's scroll bar subclass failed"));
        }
    }
    refit(list);
    Ok(())
}

fn overlay_for(list: HWND) -> *mut Overlay {
    let mut data = 0_usize;
    // SAFETY: The query only reads the subclass table of a live list.
    let found =
        unsafe { GetWindowSubclass(list, Some(list_proc), SUBCLASS_ID, Some(&raw mut data)) };
    if found.as_bool() {
        std::ptr::with_exposed_provenance_mut::<Overlay>(data)
    } else {
        std::ptr::null_mut()
    }
}

/// Turns the overlay on or off (off in high contrast) and repaints the bars
/// in the current theme.
pub(super) fn set_enabled(list: HWND, enabled: bool) {
    let pointer = overlay_for(list);
    // SAFETY: The overlay belongs to the live list on this UI thread.
    let Some(overlay) = (unsafe { pointer.as_mut() }) else {
        return;
    };
    if overlay.enabled != enabled {
        overlay.enabled = enabled;
        refit(list);
    }
    sync(overlay);
    for bar in &overlay.bars {
        // SAFETY: The bar is live or null; a null handle is ignored.
        let _ = unsafe { InvalidateRect(Some(bar.window), None, false) };
    }
}

/// The vertical and horizontal bar windows of `list`, for the smoke checks.
#[must_use]
pub(super) fn bar_windows(list: HWND) -> Option<(HWND, HWND)> {
    // SAFETY: As in `set_enabled`.
    unsafe { overlay_for(list).as_ref() }
        .map(|overlay| (overlay.bars[0].window, overlay.bars[1].window))
}

/// Asks Windows to measure the list's frame again.
fn refit(list: HWND) {
    // SAFETY: The list is live; only its frame is recomputed.
    let _ = unsafe {
        SetWindowPos(
            list,
            None,
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
}

fn is_sync_message(message: u32) -> bool {
    matches!(
        message,
        WM_PAINT
            | WM_WINDOWPOSCHANGED
            | WM_SIZE
            | WM_VSCROLL
            | WM_HSCROLL
            | WM_MOUSEWHEEL
            | WM_MOUSEHWHEEL
            | WM_KEYDOWN
            | WM_SETREDRAW
    ) || matches!(message, 0x1000..=0x10ff)
}

unsafe extern "system" fn list_proc(
    list: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    reference_data: usize,
) -> LRESULT {
    let pointer = std::ptr::with_exposed_provenance_mut::<Overlay>(reference_data);
    match message {
        WM_NCCALCSIZE => {
            // SAFETY: lparam carries the RECT or NCCALCSIZE_PARAMS of this
            // message; both start with the proposed window rectangle.
            let rect = lparam.0 as *mut RECT;
            let proposed = unsafe { rect.as_ref() }.copied();
            // SAFETY: The list's own frame calculation runs first.
            let result = unsafe { DefSubclassProc(list, message, wparam, lparam) };
            // SAFETY: The overlay outlives the subclass.
            let enabled = unsafe { pointer.as_ref() }.is_some_and(|overlay| overlay.enabled);
            if enabled && let (Some(proposed), Some(client)) = (proposed, unsafe { rect.as_mut() })
            {
                narrow_frame(list, proposed, client);
            }
            result
        }
        WM_NCDESTROY => {
            // SAFETY: This removes the exact pair being invoked; the overlay
            // is freed after its bars, and the list still gets the message.
            unsafe {
                let _ = RemoveWindowSubclass(list, Some(list_proc), subclass_id);
                if let Some(overlay) = pointer.as_mut() {
                    for bar in &mut overlay.bars {
                        if !bar.window.0.is_null() && IsWindow(Some(bar.window)).as_bool() {
                            SetWindowLongPtrW(bar.window, GWLP_USERDATA, 0);
                            let _ = DestroyWindow(bar.window);
                        }
                        bar.window = HWND::default();
                    }
                    drop(Box::from_raw(pointer));
                }
                DefSubclassProc(list, message, wparam, lparam)
            }
        }
        _ => {
            // SAFETY: Everything else stays with the list's own procedure.
            let result = unsafe { DefSubclassProc(list, message, wparam, lparam) };
            if is_sync_message(message) {
                // SAFETY: As above; `sync` guards against re-entry.
                if let Some(overlay) = unsafe { pointer.as_mut() } {
                    sync(overlay);
                }
            }
            result
        }
    }
}

/// Gives the list its whole width and height back except a thin strip for
/// each bar it shows. Windows then draws its own bars from the client edge
/// outwards, past the window's edge where nothing shows.
fn narrow_frame(list: HWND, proposed: RECT, client: &mut RECT) {
    // SAFETY: The list is live; style and DPI are only read.
    let (style, dpi) = unsafe { (GetWindowLongPtrW(list, GWL_STYLE), GetDpiForWindow(list)) };
    let strip = pixels(STRIP_DIP, dpi);
    if style & style_bit(WS_VSCROLL) != 0 {
        // SAFETY: A metric query.
        let system = unsafe { GetSystemMetricsForDpi(SM_CXVSCROLL, dpi) };
        client.right = client
            .right
            .saturating_add(system)
            .saturating_sub(strip)
            .min(proposed.right.saturating_sub(strip))
            .max(client.left);
    }
    if style & style_bit(WS_HSCROLL) != 0 {
        // SAFETY: A metric query.
        let system = unsafe { GetSystemMetricsForDpi(SM_CYHSCROLL, dpi) };
        client.bottom = client
            .bottom
            .saturating_add(system)
            .saturating_sub(strip)
            .min(proposed.bottom.saturating_sub(strip))
            .max(client.top);
    }
}

fn style_bit(style: windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE) -> isize {
    isize::try_from(style.0).unwrap_or_default()
}

/// The signed low word of a mouse message's coordinates.
fn signed_word(value: isize) -> i32 {
    let word = u16::try_from(value & 0xffff).unwrap_or_default();
    i32::from(i16::from_le_bytes(word.to_le_bytes()))
}

fn range_of(list: HWND, axis: Axis) -> ScrollRange {
    let mut info = SCROLLINFO {
        cbSize: u32::try_from(std::mem::size_of::<SCROLLINFO>()).unwrap_or_default(),
        fMask: SIF_ALL,
        ..Default::default()
    };
    let which = if axis == Axis::Vertical {
        SB_VERT
    } else {
        SB_HORZ
    };
    // SAFETY: The list is live and info is writable.
    if unsafe { GetScrollInfo(list, which, &raw mut info) }.is_err() {
        return ScrollRange::default();
    }
    ScrollRange {
        min: info.nMin,
        max: info.nMax,
        page: info.nPage,
        pos: info.nPos,
    }
}

/// Places, shows or hides, and repaints the bars to match the list. Cheap
/// when nothing changed: two scroll queries and two rectangles.
fn sync(overlay: &mut Overlay) {
    if overlay.syncing {
        return;
    }
    overlay.syncing = true;
    let list = overlay.list;
    // SAFETY: The list is live; these calls only read it.
    let (visible, style, dpi, parent) = unsafe {
        (
            IsWindowVisible(list).as_bool(),
            GetWindowLongPtrW(list, GWL_STYLE),
            GetDpiForWindow(list),
            GetParent(list).unwrap_or_default(),
        )
    };
    let mut client = RECT::default();
    // SAFETY: As above; the rectangle is writable.
    let _ = unsafe { GetClientRect(list, &raw mut client) };
    let mut origin = [POINT::default()];
    // SAFETY: Maps one point from the list to its parent.
    unsafe { MapWindowPoints(Some(list), Some(parent), &mut origin) };
    let origin = origin[0];
    let strip = pixels(STRIP_DIP, dpi);
    let vertical_shown = overlay.enabled && visible && style & style_bit(WS_VSCROLL) != 0;
    let horizontal_shown = overlay.enabled && visible && style & style_bit(WS_HSCROLL) != 0;
    let lead = header_height(list);
    for bar in &mut overlay.bars {
        let (shown, place) = match bar.axis {
            Axis::Vertical => (
                vertical_shown,
                RECT {
                    left: origin.x + client.right,
                    top: origin.y,
                    right: origin.x + client.right + strip,
                    bottom: origin.y + client.bottom + if horizontal_shown { strip } else { 0 },
                },
            ),
            Axis::Horizontal => (
                horizontal_shown,
                RECT {
                    left: origin.x,
                    top: origin.y + client.bottom,
                    right: origin.x + client.right,
                    bottom: origin.y + client.bottom + strip,
                },
            ),
        };
        let range = if shown {
            range_of(list, bar.axis)
        } else {
            ScrollRange::default()
        };
        let bar_lead = if bar.axis == Axis::Vertical { lead } else { 0 };
        if shown != bar.shown || place != bar.place {
            let flags = SWP_NOACTIVATE
                | if shown {
                    SWP_SHOWWINDOW
                } else {
                    SWP_HIDEWINDOW
                };
            // SAFETY: The bar is a live sibling of the list.
            let _ = unsafe {
                SetWindowPos(
                    bar.window,
                    Some(HWND_TOP),
                    place.left,
                    place.top,
                    place.right - place.left,
                    place.bottom - place.top,
                    flags,
                )
            };
            bar.shown = shown;
            bar.place = place;
            bar.range = range;
            bar.lead = bar_lead;
            // SAFETY: As above.
            let _ = unsafe { InvalidateRect(Some(bar.window), None, false) };
        } else if range != bar.range || bar_lead != bar.lead {
            bar.range = range;
            bar.lead = bar_lead;
            // SAFETY: As above.
            let _ = unsafe { InvalidateRect(Some(bar.window), None, false) };
        }
    }
    overlay.syncing = false;
}

fn header_height(list: HWND) -> i32 {
    // SAFETY: LVM_GETHEADER carries no pointers.
    let header = HWND(unsafe { SendMessageW(list, LVM_GETHEADER, None, None) }.0 as *mut c_void);
    // SAFETY: The header is live or null.
    if header.0.is_null() || !unsafe { IsWindowVisible(header) }.as_bool() {
        return 0;
    }
    let mut rect = RECT::default();
    // SAFETY: The rectangle is writable.
    let _ = unsafe { GetWindowRect(header, &raw mut rect) };
    (rect.bottom - rect.top).max(0)
}

/// The track a bar's thumb runs along: start and length in the bar's own
/// pixels along its axis.
fn track(bar: &Bar, dpi: u32) -> (i32, i32) {
    let inset = pixels(END_INSET_DIP, dpi);
    let length = match bar.axis {
        Axis::Vertical => bar.place.bottom - bar.place.top - bar.lead,
        Axis::Horizontal => bar.place.right - bar.place.left,
    };
    let start = if bar.axis == Axis::Vertical {
        bar.lead
    } else {
        0
    } + inset;
    (start, (length - inset * 2).max(0))
}

fn thumb_of(bar: &Bar, dpi: u32) -> Option<(i32, Thumb)> {
    let (start, length) = track(bar, dpi);
    thumb(bar.range, length, pixels(MIN_THUMB_DIP, dpi)).map(|thumb| (start, thumb))
}

fn along(bar: &Bar, lparam: LPARAM) -> i32 {
    let x = signed_word(lparam.0);
    let y = signed_word(lparam.0 >> 16);
    if bar.axis == Axis::Vertical { y } else { x }
}

/// Scrolls the list so its first visible unit is `pos`.
fn scroll_to(list: HWND, axis: Axis, pos: i32) {
    let current = range_of(list, axis).pos;
    let delta = i64::from(pos) - i64::from(current);
    if delta == 0 {
        return;
    }
    let (dx, dy) = match axis {
        Axis::Horizontal => (delta, 0),
        // Report view scrolls whole rows: pixels are rows times row height.
        Axis::Vertical => (0, delta * i64::from(row_height(list))),
    };
    let clamp =
        |value: i64| i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX });
    // SAFETY: LVM_SCROLL carries two pixel counts.
    unsafe {
        SendMessageW(
            list,
            LVM_SCROLL,
            Some(WPARAM(
                isize::try_from(clamp(dx))
                    .unwrap_or_default()
                    .cast_unsigned(),
            )),
            Some(LPARAM(clamp(dy) as isize)),
        );
    }
}

fn row_height(list: HWND) -> i32 {
    // SAFETY: LVM_GETTOPINDEX carries no pointers.
    let top = unsafe { SendMessageW(list, LVM_GETTOPINDEX, None, None) }.0;
    let mut rect = RECT {
        left: i32::try_from(LVIR_BOUNDS).unwrap_or_default(),
        ..Default::default()
    };
    // SAFETY: The rectangle is writable and outlives the synchronous call.
    unsafe {
        SendMessageW(
            list,
            LVM_GETITEMRECT,
            Some(WPARAM(top.max(0).unsigned_abs())),
            Some(LPARAM((&raw mut rect) as isize)),
        );
    }
    (rect.bottom - rect.top).max(1)
}

fn page(list: HWND, axis: Axis, direction: i32) {
    let message = if axis == Axis::Vertical {
        WM_VSCROLL
    } else {
        WM_HSCROLL
    };
    let code = if direction < 0 {
        SB_PAGEUP.0
    } else {
        SB_PAGEDOWN.0
    };
    // SAFETY: Scroll codes carry no pointers.
    unsafe {
        SendMessageW(
            list,
            message,
            Some(WPARAM(usize::try_from(code).unwrap_or_default())),
            Some(LPARAM(0)),
        );
        SendMessageW(
            list,
            message,
            Some(WPARAM(SB_ENDSCROLL.0 as usize)),
            Some(LPARAM(0)),
        );
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one window procedure handles the bar's paint and mouse in one place"
)]
unsafe extern "system" fn bar_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: The pointer slot holds the overlay or zero (see `install`).
    let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut Overlay;
    // SAFETY: The overlay lives on this UI thread for as long as the slot
    // holds it.
    let Some(overlay) = (unsafe { pointer.as_mut() }) else {
        // SAFETY: Without an overlay the bar is a plain window.
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    };
    // SAFETY: The control id is read from the live bar.
    let index = usize::from(unsafe { GetWindowLongPtrW(window, GWLP_ID) } == HORIZONTAL_ID);
    let list = overlay.list;
    // SAFETY: The list is live while its overlay is.
    let dpi = unsafe { GetDpiForWindow(list) };
    match message {
        WM_PAINT => {
            paint(overlay, index, dpi);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_MOUSEACTIVATE => LRESULT(isize::try_from(MA_NOACTIVATE).unwrap_or_default()),
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let bar = &mut overlay.bars[index];
            let at = along(bar, lparam);
            if let Some((start, thumb)) = thumb_of(bar, dpi) {
                let at = at - start;
                match page_direction(thumb, at) {
                    0 => bar.grab = Some(at - thumb.start),
                    direction => {
                        bar.paging = direction;
                        bar.held_at = at;
                        page(list, bar.axis, direction);
                        // SAFETY: The timer belongs to this bar.
                        unsafe { SetTimer(Some(window), REPEAT_TIMER, REPEAT_FIRST_MS, None) };
                    }
                }
                // SAFETY: The bar takes the mouse until the button goes up.
                unsafe { SetCapture(window) };
                // SAFETY: As above.
                let _ = unsafe { InvalidateRect(Some(window), None, false) };
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let bar = &mut overlay.bars[index];
            if !bar.hot {
                bar.hot = true;
                let mut track = TRACKMOUSEEVENT {
                    cbSize: u32::try_from(std::mem::size_of::<TRACKMOUSEEVENT>())
                        .unwrap_or_default(),
                    dwFlags: TME_LEAVE,
                    hwndTrack: window,
                    dwHoverTime: 0,
                };
                // SAFETY: The structure outlives the call.
                let _ = unsafe { TrackMouseEvent(&raw mut track) };
                // SAFETY: As above.
                let _ = unsafe { InvalidateRect(Some(window), None, false) };
            }
            let at = along(bar, lparam);
            if let Some(grab) = bar.grab
                && let Some((start, thumb)) = thumb_of(bar, dpi)
            {
                let (_, length) = track(bar, dpi);
                let pos = position_at(bar.range, length, thumb.length, at - start - grab);
                scroll_to(list, bar.axis, pos);
            } else if bar.paging != 0 {
                bar.held_at = at - track(bar, dpi).0;
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == REPEAT_TIMER => {
            let bar = &mut overlay.bars[index];
            let direction = bar.paging;
            let still = direction != 0
                && thumb_of(bar, dpi)
                    .is_some_and(|(_, thumb)| page_direction(thumb, bar.held_at) == direction);
            if still {
                page(list, bar.axis, direction);
                // SAFETY: The timer belongs to this bar.
                unsafe { SetTimer(Some(window), REPEAT_TIMER, REPEAT_MS, None) };
            } else {
                // SAFETY: As above.
                let _ = unsafe { KillTimer(Some(window), REPEAT_TIMER) };
            }
            LRESULT(0)
        }
        WM_LBUTTONUP | WM_CAPTURECHANGED => {
            let bar = &mut overlay.bars[index];
            bar.grab = None;
            bar.paging = 0;
            // SAFETY: The timer and capture belong to this bar.
            unsafe {
                let _ = KillTimer(Some(window), REPEAT_TIMER);
                if message == WM_LBUTTONUP && GetCapture() == window {
                    let _ = ReleaseCapture();
                }
                let _ = InvalidateRect(Some(window), None, false);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            overlay.bars[index].hot = false;
            // SAFETY: As above.
            let _ = unsafe { InvalidateRect(Some(window), None, false) };
            LRESULT(0)
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            // SAFETY: The wheel goes to the list as if it were over it.
            unsafe { SendMessageW(list, message, Some(wparam), Some(lparam)) }
        }
        WM_NCDESTROY => {
            overlay.bars[index].window = HWND::default();
            // SAFETY: The bar no longer reaches the overlay.
            unsafe {
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                DefWindowProcW(window, message, wparam, lparam)
            }
        }
        // SAFETY: Everything else is a plain window's.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

fn paint(overlay: &Overlay, index: usize, dpi: u32) {
    let bar = &overlay.bars[index];
    let window = bar.window;
    let mut paint = PAINTSTRUCT::default();
    // SAFETY: The bar is handling WM_PAINT; paint is writable until EndPaint.
    let device = unsafe { BeginPaint(window, &raw mut paint) };
    let state = super::state_pointer_for(overlay.owner);
    // SAFETY: The owner's state outlives its child windows' paints.
    if let Some(state) = unsafe { state.as_ref() }
        && !device.0.is_null()
    {
        draw(device, bar, &state.theme, dpi);
    }
    // SAFETY: Closes the BeginPaint above.
    let _ = unsafe { EndPaint(window, &raw const paint) };
}

fn draw(device: windows::Win32::Graphics::Gdi::HDC, bar: &Bar, theme: &ThemeResources, dpi: u32) {
    let width = bar.place.right - bar.place.left;
    let height = bar.place.bottom - bar.place.top;
    let whole = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    let palette = theme.palette;
    // SAFETY: The theme's brushes outlive this paint; the stock DC brush and
    // pen are always valid.
    unsafe {
        FillRect(device, &raw const whole, theme.brushes.surface());
        if bar.lead > 0 {
            let lead = RECT {
                bottom: bar.lead.min(height),
                ..whole
            };
            FillRect(device, &raw const lead, theme.brushes.surface_muted());
            let rule = RECT {
                top: lead.bottom - theme.metrics.border_width,
                ..lead
            };
            FillRect(device, &raw const rule, theme.brushes.border());
        }
        let Some((start, thumb)) = thumb_of(bar, dpi) else {
            return;
        };
        let active = bar.hot || bar.grab.is_some();
        let thickness = pixels(if active { THUMB_HOT_DIP } else { THUMB_DIP }, dpi);
        let colour = if active {
            palette.text_secondary
        } else {
            palette.text_disabled
        };
        let across = match bar.axis {
            Axis::Vertical => width,
            Axis::Horizontal => height,
        };
        let near = (across - thickness) / 2;
        let (from, to) = (start + thumb.start, start + thumb.end());
        let rect = match bar.axis {
            Axis::Vertical => RECT {
                left: near,
                top: from,
                right: near + thickness,
                bottom: to,
            },
            Axis::Horizontal => RECT {
                left: from,
                top: near,
                right: to,
                bottom: near + thickness,
            },
        };
        let previous_brush = SelectObject(device, GetStockObject(DC_BRUSH));
        let previous_pen = SelectObject(device, GetStockObject(DC_PEN));
        SetDCBrushColor(device, colour.colorref());
        SetDCPenColor(device, colour.colorref());
        let _ = RoundRect(
            device,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            thickness,
            thickness,
        );
        SelectObject(device, previous_pen);
        SelectObject(device, previous_brush);
    }
}
