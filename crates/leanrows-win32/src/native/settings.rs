//! The settings panel: theme, zoom and every keyboard shortcut, as in
//! `LeanMark`.
//!
//! The panel is a child window on the right edge below the chrome. Its
//! controls are ordinary buttons, radio buttons, labels and a read-only list,
//! so they keep their native roles and keyboard behaviour. The panel forwards
//! their commands, notifications and colour requests to the main window, which
//! paints them with the shared theme.

use std::ffi::c_void;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER,
    DrawTextW, EndPaint, FillRect, HDC, HGDIOBJ, PAINTSTRUCT, SelectObject, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Controls::{
    CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDRF_NOTIFYITEMDRAW, CDRF_SKIPDEFAULT, HIMAGELIST,
    ILC_COLOR32, ImageList_Create, ImageList_Destroy, LVCF_SUBITEM, LVCF_WIDTH, LVCOLUMNW,
    LVIF_PARAM, LVIF_TEXT, LVITEMW, LVM_DELETEALLITEMS, LVM_GETITEMW, LVM_INSERTCOLUMNW,
    LVM_INSERTITEMW, LVM_SETBKCOLOR, LVM_SETCOLUMNWIDTH, LVM_SETIMAGELIST, LVM_SETITEMW,
    LVS_NOCOLUMNHEADER, LVS_REPORT, LVS_SINGLESEL, LVSIL_SMALL, NMLVCUSTOMDRAW, WC_LISTVIEWW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BS_AUTORADIOBUTTON, BS_OWNERDRAW, CreateWindowExW, DefWindowProcW, GetClientRect, GetParent,
    IDC_ARROW, LoadCursorW, RegisterClassExW, SendMessageW, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_COMMAND, WM_CTLCOLORBTN, WM_CTLCOLORSTATIC, WM_DRAWITEM, WM_ERASEBKGND, WM_NOTIFY, WM_PAINT,
    WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN, WS_GROUP, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{PCWSTR, PWSTR, w};

use super::layout::UiRect;
use super::theme::{ThemeMode, ThemeResources};
use crate::ShellError;
use crate::shortcuts::{SHORTCUTS, groups};

const PANEL_CLASS_NAME: PCWSTR = w!("LeanRows.SettingsPanel");
/// The close button.
pub(super) const ID_SETTINGS_CLOSE: u16 = 127;
/// The five theme choices, in `ThemeMode::ALL` order.
pub(super) const ID_SETTINGS_THEME_FIRST: u16 = 161;
pub(super) const ID_SETTINGS_THEME_LAST: u16 = 165;
/// Item data marking a group heading in the shortcut list.
const HEADING: isize = 1;

/// Every child of the panel. All handles are null until `create` runs.
#[derive(Default)]
pub(super) struct SettingsHandles {
    pub(super) panel: HWND,
    pub(super) title: HWND,
    pub(super) close: HWND,
    pub(super) theme_label: HWND,
    pub(super) themes: [HWND; 5],
    pub(super) zoom_label: HWND,
    pub(super) zoom_out: HWND,
    pub(super) zoom_value: HWND,
    pub(super) zoom_in: HWND,
    pub(super) zoom_reset: HWND,
    pub(super) shortcuts_label: HWND,
    pub(super) shortcuts: HWND,
}

impl SettingsHandles {
    /// The panel's static labels, painted on the panel's surface.
    pub(super) fn labels(&self) -> [HWND; 5] {
        [
            self.title,
            self.theme_label,
            self.zoom_label,
            self.zoom_value,
            self.shortcuts_label,
        ]
    }

    /// The panel's push buttons, owner-drawn like the toolbar's.
    pub(super) fn buttons(&self) -> [HWND; 4] {
        [self.close, self.zoom_out, self.zoom_in, self.zoom_reset]
    }

    /// The radio button for `mode`.
    pub(super) fn theme_choice(&self, mode: ThemeMode) -> HWND {
        ThemeMode::ALL
            .iter()
            .position(|candidate| *candidate == mode)
            .map_or(HWND::default(), |index| self.themes[index])
    }

    /// The theme a radio button chooses, if `control` is one.
    pub(super) fn theme_for(&self, control: HWND) -> Option<ThemeMode> {
        self.themes
            .iter()
            .position(|choice| *choice == control && !control.0.is_null())
            .map(|index| ThemeMode::ALL[index])
    }

    /// The controls Tab moves through while the panel is open, in order.
    pub(super) fn focus_order(&self, chosen: ThemeMode) -> [HWND; 6] {
        [
            self.close,
            self.theme_choice(chosen),
            self.zoom_out,
            self.zoom_in,
            self.zoom_reset,
            self.shortcuts,
        ]
    }
}

pub(super) fn register_class(instance: HINSTANCE) -> Result<(), ShellError> {
    let class = WNDCLASSEXW {
        cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap_or_default(),
        lpfnWndProc: Some(panel_proc),
        hInstance: instance,
        // SAFETY: IDC_ARROW is a predefined system cursor.
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
        lpszClassName: PANEL_CLASS_NAME,
        ..Default::default()
    };
    // SAFETY: The class structure and its static name outlive the call.
    if unsafe { RegisterClassExW(&raw const class) } == 0 {
        return Err(ShellError::new("settings panel class registration failed"));
    }
    Ok(())
}

/// Creates the hidden panel and its controls under `window`.
pub(super) fn create(instance: HINSTANCE, window: HWND) -> Result<SettingsHandles, ()> {
    let panel = child(
        instance,
        window,
        PANEL_CLASS_NAME,
        w!("Settings"),
        WS_CHILD | WS_CLIPCHILDREN,
        None,
    )?;
    let label = |text: PCWSTR| {
        child(
            instance,
            panel,
            w!("STATIC"),
            text,
            WS_CHILD | WS_VISIBLE,
            None,
        )
    };
    let button = |text: PCWSTR, id: u16| {
        child(
            instance,
            panel,
            w!("BUTTON"),
            text,
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WINDOW_STYLE(u32::try_from(BS_OWNERDRAW).unwrap_or_default()),
            Some(id),
        )
    };
    let title = label(w!("Settings"))?;
    let close = button(w!("\u{00d7}"), ID_SETTINGS_CLOSE)?;
    let theme_label = label(w!("THEME"))?;
    let mut themes = [HWND::default(); 5];
    for (index, mode) in ThemeMode::ALL.into_iter().enumerate() {
        let mut text: Vec<u16> = mode.label().encode_utf16().chain(Some(0)).collect();
        // The first radio starts the group so arrow keys stay within it.
        let group = if index == 0 {
            WS_GROUP | WS_TABSTOP
        } else {
            WINDOW_STYLE(0)
        };
        themes[index] = child(
            instance,
            panel,
            w!("BUTTON"),
            PCWSTR(text.as_mut_ptr()),
            WS_CHILD
                | WS_VISIBLE
                | group
                | WINDOW_STYLE(u32::try_from(BS_AUTORADIOBUTTON).unwrap_or_default()),
            Some(ID_SETTINGS_THEME_FIRST + u16::try_from(index).unwrap_or_default()),
        )?;
    }
    let zoom_label = label(w!("ZOOM"))?;
    let zoom_out = button(w!("\u{2212}"), super::ID_VIEW_ZOOM_OUT)?;
    let zoom_value = label(w!("100%"))?;
    let zoom_in = button(w!("+"), super::ID_VIEW_ZOOM_IN)?;
    let zoom_reset = button(w!("Reset"), super::ID_VIEW_ZOOM_RESET)?;
    let shortcuts_label = label(w!("KEYBOARD SHORTCUTS"))?;
    let shortcuts = child(
        instance,
        panel,
        WC_LISTVIEWW,
        w!("Keyboard shortcuts"),
        WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WINDOW_STYLE(LVS_REPORT | LVS_NOCOLUMNHEADER | LVS_SINGLESEL),
        None,
    )?;
    fill_shortcuts(shortcuts)?;
    Ok(SettingsHandles {
        panel,
        title,
        close,
        theme_label,
        themes,
        zoom_label,
        zoom_out,
        zoom_value,
        zoom_in,
        zoom_reset,
        shortcuts_label,
        shortcuts,
    })
}

fn child(
    instance: HINSTANCE,
    parent: HWND,
    class: PCWSTR,
    text: PCWSTR,
    style: WINDOW_STYLE,
    id: Option<u16>,
) -> Result<HWND, ()> {
    let menu = id.map(|id| {
        windows::Win32::UI::WindowsAndMessaging::HMENU(std::ptr::without_provenance_mut::<c_void>(
            usize::from(id),
        ))
    });
    // SAFETY: The parent, class, text and module belong to this UI thread.
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            text,
            style,
            0,
            0,
            0,
            0,
            Some(parent),
            menu,
            Some(instance),
            None,
        )
    }
    .map_err(|_| ())
}

/// Two columns, keys and action, with one heading row before each group.
fn fill_shortcuts(list: HWND) -> Result<(), ()> {
    for subitem in 0..2 {
        let column = LVCOLUMNW {
            mask: LVCF_WIDTH | LVCF_SUBITEM,
            cx: 120,
            iSubItem: subitem,
            ..Default::default()
        };
        // SAFETY: The column structure outlives this synchronous message.
        let inserted = unsafe {
            SendMessageW(
                list,
                LVM_INSERTCOLUMNW,
                Some(WPARAM(usize::try_from(subitem).unwrap_or_default())),
                Some(LPARAM((&raw const column) as isize)),
            )
        };
        if inserted.0 < 0 {
            return Err(());
        }
    }
    // SAFETY: LVM_DELETEALLITEMS carries no pointers.
    unsafe { SendMessageW(list, LVM_DELETEALLITEMS, None, None) };
    let mut index = 0;
    for group in groups() {
        insert_row(list, index, group, None, HEADING)?;
        index += 1;
        for entry in SHORTCUTS.iter().filter(|entry| entry.group == group) {
            insert_row(list, index, entry.keys, Some(entry.action), 0)?;
            index += 1;
        }
    }
    Ok(())
}

fn insert_row(
    list: HWND,
    index: i32,
    keys: &str,
    action: Option<&str>,
    data: isize,
) -> Result<(), ()> {
    let mut text: Vec<u16> = keys.encode_utf16().chain(Some(0)).collect();
    let item = LVITEMW {
        mask: LVIF_TEXT | LVIF_PARAM,
        iItem: index,
        pszText: PWSTR(text.as_mut_ptr()),
        lParam: LPARAM(data),
        ..Default::default()
    };
    // SAFETY: The item and its text outlive this synchronous message, which
    // copies the text.
    let inserted = unsafe {
        SendMessageW(
            list,
            LVM_INSERTITEMW,
            None,
            Some(LPARAM((&raw const item) as isize)),
        )
    };
    if inserted.0 < 0 {
        return Err(());
    }
    if let Some(action) = action {
        let mut text: Vec<u16> = action.encode_utf16().chain(Some(0)).collect();
        let subitem = LVITEMW {
            mask: LVIF_TEXT,
            iItem: index,
            iSubItem: 1,
            pszText: PWSTR(text.as_mut_ptr()),
            ..Default::default()
        };
        // SAFETY: As above.
        unsafe {
            SendMessageW(
                list,
                LVM_SETITEMW,
                None,
                Some(LPARAM((&raw const subitem) as isize)),
            );
        }
    }
    Ok(())
}

/// Sizes the shortcut rows. A one-pixel-wide image list of the row height
/// is the documented way to set a report list's row height.
pub(super) fn apply_row_height(list: HWND, height: i32) {
    // SAFETY: The image list is handed to the list, which destroys it with
    // itself because the list does not share image lists.
    unsafe {
        let images = ImageList_Create(1, height.max(1), ILC_COLOR32, 0, 0);
        if !images.is_invalid() {
            let previous = SendMessageW(
                list,
                LVM_SETIMAGELIST,
                Some(WPARAM(usize::try_from(LVSIL_SMALL).unwrap_or_default())),
                Some(LPARAM(images.0)),
            );
            // The list hands back the list it held before; that one is ours.
            if previous.0 != 0 {
                let _ = ImageList_Destroy(Some(HIMAGELIST(previous.0)));
            }
        }
    }
}

/// Places the panel's controls inside `bounds`, `LeanMark`'s padding of 16
/// above and 20 at the sides, with 20 between groups.
#[allow(
    clippy::too_many_lines,
    reason = "one top-to-bottom pass places every control in reading order"
)]
pub(super) fn layout(handles: &SettingsHandles, bounds: UiRect, theme: &ThemeResources) {
    let m = theme.metrics;
    let side = m.space_4.saturating_add(m.space_1);
    let left = side;
    let right = bounds.width.saturating_sub(side).max(left);
    let width = right.saturating_sub(left);
    let mut y = m.space_4;
    let place = |handle: HWND, x: i32, y: i32, w: i32, h: i32| {
        // SAFETY: Every handle is a live child of the panel.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::MoveWindow(
                handle,
                x,
                y,
                w.max(0),
                h.max(0),
                true,
            );
        }
    };
    let title_height = m.control_target.min(m.space_6.saturating_add(m.space_2));
    place(
        handles.title,
        left,
        y,
        width.saturating_sub(title_height),
        title_height,
    );
    place(
        handles.close,
        right.saturating_sub(title_height),
        y,
        title_height,
        title_height,
    );
    y = y.saturating_add(title_height).saturating_add(m.space_3);
    let label_height = m.space_4.saturating_add(m.space_1);
    place(handles.theme_label, left, y, width, label_height);
    y = y.saturating_add(label_height).saturating_add(m.space_2);
    let choice_height = m.space_6.saturating_add(m.space_2);
    for choice in handles.themes {
        place(choice, left, y, width, choice_height);
        y = y.saturating_add(choice_height);
    }
    y = y.saturating_add(m.space_4.saturating_add(m.space_1));
    place(handles.zoom_label, left, y, width, label_height);
    y = y.saturating_add(label_height).saturating_add(m.space_2);
    let button = choice_height;
    let value_width = m.space_6.saturating_mul(2);
    let mut x = left;
    place(handles.zoom_out, x, y, button, button);
    x = x.saturating_add(button).saturating_add(m.space_2);
    place(handles.zoom_value, x, y, value_width, button);
    x = x.saturating_add(value_width).saturating_add(m.space_2);
    place(handles.zoom_in, x, y, button, button);
    x = x.saturating_add(button).saturating_add(m.space_2);
    place(
        handles.zoom_reset,
        x,
        y,
        m.space_6.saturating_mul(3),
        button,
    );
    y = y
        .saturating_add(button)
        .saturating_add(m.space_4.saturating_add(m.space_1));
    place(handles.shortcuts_label, left, y, width, label_height);
    y = y.saturating_add(label_height).saturating_add(m.space_1);
    let list_height = bounds.height.saturating_sub(y).saturating_sub(m.space_6);
    place(handles.shortcuts, left, y, width, list_height);
    let row_width = shortcut_row_width(width, m.dpi);
    for (column, column_width) in [(0, row_width), (1, 0)] {
        // SAFETY: LVM_SETCOLUMNWIDTH carries a column index and a width.
        unsafe {
            SendMessageW(
                handles.shortcuts,
                LVM_SETCOLUMNWIDTH,
                Some(WPARAM(column)),
                Some(LPARAM(isize::try_from(column_width).unwrap_or_default())),
            );
        }
    }
    fit_rows(handles.shortcuts, theme);
    // SAFETY: The list colour message carries a COLORREF value only.
    unsafe {
        SendMessageW(
            handles.shortcuts,
            LVM_SETBKCOLOR,
            Some(WPARAM(0)),
            Some(LPARAM(
                isize::try_from(theme.palette.surface.colorref().0).unwrap_or_default(),
            )),
        );
    }
}

/// Paints one shortcut row: group headings as small secondary text, and
/// entries as the keys in a bordered box with the action right-aligned and a
/// rule below, as `LeanMark`'s `.shortcut-row`. Returns `None` when the
/// notification is not for the shortcut list.
#[allow(
    clippy::too_many_lines,
    reason = "one custom-draw stage paints both kinds of row in reading order"
)]
pub(super) fn draw_shortcut(theme: &ThemeResources, list: HWND, lparam: LPARAM) -> Option<LRESULT> {
    // SAFETY: NM_CUSTOMDRAW from a list view carries an NMLVCUSTOMDRAW.
    let draw = unsafe { &*(lparam.0 as *const NMLVCUSTOMDRAW) };
    if draw.nmcd.hdr.hwndFrom != list {
        return None;
    }
    if draw.nmcd.dwDrawStage == CDDS_PREPAINT {
        return Some(LRESULT(
            isize::try_from(CDRF_NOTIFYITEMDRAW).unwrap_or_default(),
        ));
    }
    if draw.nmcd.dwDrawStage != CDDS_ITEMPREPAINT {
        return None;
    }
    let index = i32::try_from(draw.nmcd.dwItemSpec).unwrap_or_default();
    let mut keys = [0_u16; 64];
    let mut action = [0_u16; 64];
    let heading = item_text(list, index, 0, &mut keys) == HEADING;
    let _ = item_text(list, index, 1, &mut action);
    let keys_length = length(&keys);
    let action_length = length(&action);
    let device = draw.nmcd.hdc;
    let row = draw.nmcd.rc;
    let palette = theme.palette;
    let m = theme.metrics;
    fill(device, row, theme.brushes.surface());
    // SAFETY: The theme owns every font and brush; the device belongs to
    // this custom-draw stage and the text buffers are local.
    unsafe {
        SetBkMode(device, TRANSPARENT);
        if heading {
            SelectObject(device, HGDIOBJ(theme.fonts.semibold().0));
            SetTextColor(device, palette.text_secondary.colorref());
            let mut text_rect = RECT {
                top: row.top.saturating_add(m.space_2),
                ..row
            };
            DrawTextW(
                device,
                &mut keys[..keys_length],
                &raw mut text_rect,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
        } else {
            SelectObject(device, HGDIOBJ(theme.fonts.caption().0));
            let mut size = windows::Win32::Foundation::SIZE::default();
            let key_text = &keys[..keys_length];
            let _ = windows::Win32::Graphics::Gdi::GetTextExtentPoint32W(
                device,
                key_text,
                &raw mut size,
            );
            let pad = m.space_1;
            let box_height = size.cy.saturating_add(pad);
            let top = row.top.saturating_add(
                row.bottom
                    .saturating_sub(row.top)
                    .saturating_sub(box_height)
                    / 2,
            );
            let key_box = RECT {
                left: row.left,
                top,
                right: row
                    .left
                    .saturating_add(size.cx)
                    .saturating_add(pad.saturating_mul(2))
                    .min(row.right),
                bottom: top.saturating_add(box_height),
            };
            fill(device, key_box, theme.brushes.border());
            let inner = RECT {
                left: key_box.left.saturating_add(1),
                top: key_box.top.saturating_add(1),
                right: key_box.right.saturating_sub(1),
                bottom: key_box.bottom.saturating_sub(1),
            };
            fill(device, inner, theme.brushes.surface_muted());
            SetTextColor(device, palette.text_primary.colorref());
            let mut key_rect = RECT {
                left: key_box.left.saturating_add(pad),
                ..key_box
            };
            DrawTextW(
                device,
                &mut keys[..keys_length],
                &raw mut key_rect,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
            SelectObject(device, HGDIOBJ(theme.fonts.body().0));
            SetTextColor(device, palette.text_secondary.colorref());
            let left = key_box.right.saturating_add(m.space_3);
            let height = action_height(device, &mut action[..action_length], row.right - left);
            let top = row.top + (row.bottom - row.top - height) / 2;
            let mut action_rect = RECT {
                left,
                top,
                right: row.right,
                bottom: top + height,
            };
            DrawTextW(
                device,
                &mut action[..action_length],
                &raw mut action_rect,
                ACTION_FORMAT,
            );
            let rule = RECT {
                top: row.bottom.saturating_sub(m.border_width.max(1)),
                ..row
            };
            fill(device, rule, theme.brushes.border());
        }
    }
    Some(LRESULT(
        isize::try_from(CDRF_SKIPDEFAULT).unwrap_or_default(),
    ))
}

/// The shortcut rows' width in a list `width` pixels wide: all of it but the
/// room a vertical bar takes, so no row runs under the bar and none needs a
/// horizontal bar. The list decides on a horizontal bar as if its vertical
/// bar were the system's width, so the rows leave that much, which is more
/// than the thin bar's strip.
#[must_use]
pub(super) fn shortcut_row_width(width: i32, dpi: u32) -> i32 {
    // SAFETY: A metric query.
    let system = unsafe {
        windows::Win32::UI::HiDpi::GetSystemMetricsForDpi(
            windows::Win32::UI::WindowsAndMessaging::SM_CXVSCROLL,
            dpi,
        )
    };
    let strip = super::scrollbar::pixels(super::scrollbar::STRIP_DIP, dpi);
    width.saturating_sub(system.max(strip)).max(0)
}

const ACTION_FORMAT: windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT =
    windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT(
        DT_RIGHT.0 | windows::Win32::Graphics::Gdi::DT_WORDBREAK.0 | DT_NOPREFIX.0,
    );

/// The height `text` takes wrapped at `width` pixels in the font selected
/// into `device`.
fn action_height(device: HDC, text: &mut [u16], width: i32) -> i32 {
    let mut rect = RECT {
        right: width.max(1),
        ..Default::default()
    };
    // SAFETY: The rectangle and the text are writable for the call.
    unsafe {
        DrawTextW(
            device,
            text,
            &raw mut rect,
            ACTION_FORMAT | windows::Win32::Graphics::Gdi::DT_CALCRECT,
        );
    }
    rect.bottom - rect.top
}

/// The height each shortcut's row needs, in reading order, when rows are
/// `row_width` pixels wide: the action wrapped beside its keys' box, with
/// the usual padding above and below, and never less than a table row.
pub(super) fn shortcut_row_heights(list: HWND, theme: &ThemeResources, row_width: i32) -> Vec<i32> {
    let m = theme.metrics;
    // SAFETY: The list is live; the screen-compatible DC is released below.
    let device = unsafe { windows::Win32::Graphics::Gdi::GetDC(Some(list)) };
    if device.0.is_null() {
        return Vec::new();
    }
    let mut fits = Vec::with_capacity(SHORTCUTS.len());
    for entry in &SHORTCUTS {
        let keys: Vec<u16> = entry.keys.encode_utf16().collect();
        let mut action: Vec<u16> = entry.action.encode_utf16().collect();
        let mut size = windows::Win32::Foundation::SIZE::default();
        // SAFETY: The fonts outlive the DC; the buffers are local.
        unsafe {
            SelectObject(device, HGDIOBJ(theme.fonts.caption().0));
            let _ =
                windows::Win32::Graphics::Gdi::GetTextExtentPoint32W(device, &keys, &raw mut size);
            SelectObject(device, HGDIOBJ(theme.fonts.body().0));
        }
        let keys_box = size.cx.saturating_add(m.space_1.saturating_mul(2));
        let action_room = row_width - keys_box - m.space_3;
        let mut rect = RECT {
            right: action_room.max(1),
            ..Default::default()
        };
        // SAFETY: As in `action_height`.
        unsafe {
            DrawTextW(
                device,
                &mut action,
                &raw mut rect,
                ACTION_FORMAT | windows::Win32::Graphics::Gdi::DT_CALCRECT,
            );
        }
        fits.push(
            m.table_row_height
                .max((rect.bottom - rect.top).saturating_add(m.space_2.saturating_mul(2))),
        );
    }
    // SAFETY: Releases the DC taken above.
    unsafe {
        windows::Win32::Graphics::Gdi::ReleaseDC(Some(list), device);
    }
    fits
}

/// Gives every row the height the tallest wrapped action needs, and leaves
/// the rows alone when they already have it.
pub(super) fn fit_rows(list: HWND, theme: &ThemeResources) {
    let mut rect = RECT::default();
    // SAFETY: The list is live and the rectangle writable.
    if unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(list, &raw mut rect) }
        .is_err()
    {
        return;
    }
    let row_width = shortcut_row_width(rect.right - rect.left, theme.metrics.dpi);
    let height = shortcut_row_heights(list, theme, row_width)
        .into_iter()
        .max()
        .unwrap_or(theme.metrics.table_row_height);
    if row_height(list) != height {
        apply_row_height(list, height);
    }
}

/// The height of the list's rows now.
#[must_use]
pub(super) fn row_height(list: HWND) -> i32 {
    let mut rect = RECT {
        left: i32::try_from(windows::Win32::UI::Controls::LVIR_BOUNDS).unwrap_or_default(),
        ..Default::default()
    };
    // SAFETY: The rectangle outlives the synchronous call.
    unsafe {
        SendMessageW(
            list,
            windows::Win32::UI::Controls::LVM_GETITEMRECT,
            Some(WPARAM(0)),
            Some(LPARAM((&raw mut rect) as isize)),
        );
    }
    rect.bottom - rect.top
}

/// Reads one cell's text and returns the row's item data.
fn item_text(list: HWND, index: i32, subitem: i32, out: &mut [u16]) -> isize {
    let mut item = LVITEMW {
        mask: LVIF_TEXT | LVIF_PARAM,
        iItem: index,
        iSubItem: subitem,
        pszText: PWSTR(out.as_mut_ptr()),
        cchTextMax: i32::try_from(out.len()).unwrap_or_default(),
        ..Default::default()
    };
    // SAFETY: The item and its writable buffer outlive the synchronous call.
    unsafe {
        SendMessageW(
            list,
            LVM_GETITEMW,
            None,
            Some(LPARAM((&raw mut item) as isize)),
        );
    }
    item.lParam.0
}

fn length(text: &[u16]) -> usize {
    text.iter()
        .position(|unit| *unit == 0)
        .unwrap_or(text.len())
}

fn fill(device: HDC, rect: RECT, brush: windows::Win32::Graphics::Gdi::HBRUSH) {
    // SAFETY: The device and brush are live for the current paint.
    unsafe {
        FillRect(device, &raw const rect, brush);
    }
}

/// Paints the panel's surface and its left border, and hands its controls'
/// messages to the main window, which owns the theme.
unsafe extern "system" fn panel_proc(
    panel: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_COMMAND | WM_NOTIFY | WM_DRAWITEM | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            // SAFETY: The panel's parent is the live main window.
            let Ok(parent) = (unsafe { GetParent(panel) }) else {
                // SAFETY: Default processing for this window.
                return unsafe { DefWindowProcW(panel, message, wparam, lparam) };
            };
            // SAFETY: The message is forwarded unchanged to the same thread.
            unsafe { SendMessageW(parent, message, Some(wparam), Some(lparam)) }
        }
        WM_ERASEBKGND => {
            // SAFETY: The panel's parent is the live main window.
            let parent = unsafe { GetParent(panel) }.unwrap_or_default();
            let device = HDC(wparam.0 as *mut c_void);
            LRESULT(isize::from(super::paint_settings_surface(
                parent, panel, device,
            )))
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            // SAFETY: The surface is painted in WM_ERASEBKGND; this validates
            // the update region.
            unsafe {
                let _ = BeginPaint(panel, &raw mut paint);
                let _ = EndPaint(panel, &raw const paint);
            }
            LRESULT(0)
        }
        // SAFETY: Default processing for every other message.
        _ => unsafe { DefWindowProcW(panel, message, wparam, lparam) },
    }
}

/// Fills the panel with the surface colour and draws its left border.
pub(super) fn paint_surface(theme: &ThemeResources, panel: HWND, device: HDC) -> bool {
    let mut client = RECT::default();
    // SAFETY: The panel is live and RECT is writable.
    if unsafe { GetClientRect(panel, &raw mut client) }.is_err() {
        return false;
    }
    fill(device, client, theme.brushes.surface());
    let border = RECT {
        right: client
            .left
            .saturating_add(theme.metrics.border_width.max(1)),
        ..client
    };
    fill(device, border, theme.brushes.border());
    true
}
