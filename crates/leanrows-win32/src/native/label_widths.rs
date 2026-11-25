//! Measures every fixed-width label in every interface language with the
//! fonts the controls really use, and fails when one would be clipped.
//!
//! The controls keep `LeanMark`'s fixed widths, so this is where a longer
//! Chinese or Japanese label is caught. GDI's `DrawTextW` measures through
//! Windows font linking, the same path the controls paint with, so the CJK
//! fallback fonts are measured too.

use windows::Win32::Foundation::{RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DT_CALCRECT, DT_NOPREFIX, DT_SINGLELINE, DeleteObject, DrawTextW, GetDC,
    GetTextExtentPoint32W, HDC, HFONT, HGDIOBJ, LOGFONTW, ReleaseDC, SelectObject,
};
use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
use windows::Win32::UI::WindowsAndMessaging::SM_CXMENUCHECK;

use super::layout::UiMetrics;
use super::theme::{ThemeMode, ThemeResources};
use crate::localization::{Language, LanguageSetting, Strings, strings_for};

/// The width `text` takes on one line in `font`. An `&` marks an access key
/// and takes no room, as in a menu or dialog.
fn text_width(device: HDC, font: HFONT, text: &str) -> i32 {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    let mut rect = RECT::default();
    let prefix = if text.contains('&') {
        DT_SINGLELINE
    } else {
        DT_SINGLELINE | DT_NOPREFIX
    };
    // SAFETY: The device is the screen DC taken by the test, the font is
    // live for the call, and the buffer and rectangle are local.
    unsafe {
        let previous = SelectObject(device, HGDIOBJ(font.0));
        DrawTextW(device, &mut units, &raw mut rect, prefix | DT_CALCRECT);
        SelectObject(device, previous);
    }
    rect.right - rect.left
}

/// The dialog's horizontal base unit for "Segoe UI" 9 pt, measured the way
/// Windows documents it, and the font itself.
fn dialog_font(device: HDC, dpi: u32) -> (HFONT, i32) {
    let mut face = [0_u16; 32];
    for (slot, unit) in face.iter_mut().zip("Segoe UI".encode_utf16()) {
        *slot = unit;
    }
    let definition = LOGFONTW {
        lfHeight: -i32::try_from(dpi * 9 / 72).unwrap_or(12),
        lfFaceName: face,
        ..Default::default()
    };
    // SAFETY: The definition outlives the call; the caller deletes the font.
    let font = unsafe { CreateFontIndirectW(&raw const definition) };
    let alphabet: Vec<u16> = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"
        .encode_utf16()
        .collect();
    let mut size = SIZE::default();
    // SAFETY: As in `text_width`.
    unsafe {
        let previous = SelectObject(device, HGDIOBJ(font.0));
        let _ = GetTextExtentPoint32W(device, &alphabet, &raw mut size);
        SelectObject(device, previous);
    }
    (font, (size.cx / 26 + 1) / 2)
}

struct Check<'a> {
    key: &'static str,
    text: &'a str,
    room: i32,
    font: HFONT,
}

fn scale(value: i32, dpi: u32) -> i32 {
    value * i32::try_from(dpi).unwrap_or(96) / 96
}

#[allow(
    clippy::too_many_lines,
    reason = "one list names every fixed-width label and its room"
)]
fn checks<'a>(
    strings: &'a Strings,
    resources: &ThemeResources,
    layout: UiMetrics,
    dialog: (HFONT, i32),
    dpi: u32,
) -> Vec<Check<'a>> {
    let fonts = &resources.fonts;
    let m = resources.metrics;
    let pad = m.space_2.saturating_mul(2);
    // SAFETY: A metric query.
    let radio = unsafe { GetSystemMetricsForDpi(SM_CXMENUCHECK, dpi) } + m.space_2;
    // The panel's content: its width less 20 DIP each side.
    let panel = layout.settings_panel_width - (m.space_4 + m.space_1) * 2;
    let title_height = m.control_target.min(m.space_6 + m.space_2);
    let theme_room = layout.theme_button_width - m.swatch_size - m.space_2 - pad;
    let (dialog_font, base) = dialog;
    let dlu = |units: i32| units * base / 4;
    let mut list = vec![
        Check {
            key: "open_button",
            text: strings.open_button,
            room: layout.open_button_width - pad,
            font: fonts.semibold(),
        },
        Check {
            key: "reload_button",
            text: strings.reload_button,
            room: layout.wide_reload_width - pad,
            font: fonts.semibold(),
        },
        Check {
            key: "goto_button",
            text: strings.goto_button,
            room: layout.wide_goto_width - pad,
            font: fonts.semibold(),
        },
        Check {
            key: "empty_open_button",
            text: strings.empty_open_button,
            room: scale(168, dpi) - pad,
            font: fonts.semibold(),
        },
        Check {
            key: "settings_zoom_reset",
            text: strings.settings_zoom_reset,
            room: m.space_6 * 3 - pad,
            font: fonts.semibold(),
        },
        Check {
            key: "settings_title",
            text: strings.settings_title,
            room: panel - title_height,
            font: fonts.title(),
        },
        Check {
            key: "row_column",
            text: strings.row_column,
            room: scale(112, dpi) - m.space_3 * 2,
            font: fonts.grid_header(),
        },
        Check {
            key: "find_cue",
            text: strings.find_cue,
            room: layout.narrow_search_min_width - pad,
            font: fonts.body(),
        },
        Check {
            key: "goto_label",
            text: strings.goto_label,
            room: dlu(214),
            font: dialog_font,
        },
        Check {
            key: "goto_ok",
            text: strings.goto_ok,
            room: dlu(52) - pad,
            font: dialog_font,
        },
        Check {
            key: "goto_cancel",
            text: strings.goto_cancel,
            room: dlu(52) - pad,
            font: dialog_font,
        },
    ];
    for (key, text) in [
        ("theme_system", strings.theme_system),
        ("theme_light", strings.theme_light),
        ("theme_dark", strings.theme_dark),
        ("theme_dim", strings.theme_dim),
        ("theme_dusk", strings.theme_dusk),
    ] {
        list.push(Check {
            key,
            text,
            room: theme_room,
            font: fonts.semibold(),
        });
        list.push(Check {
            key,
            text,
            room: panel - radio,
            font: fonts.body(),
        });
    }
    list.push(Check {
        key: "language_system",
        text: strings.language_system,
        room: panel - radio,
        font: fonts.body(),
    });
    for (key, text) in [
        ("settings_theme", strings.settings_theme),
        ("settings_zoom", strings.settings_zoom),
        ("settings_language", strings.settings_language),
        ("settings_shortcuts", strings.settings_shortcuts),
    ] {
        list.push(Check {
            key,
            text,
            room: panel,
            font: fonts.legend(),
        });
    }
    for (key, text) in [
        ("gesture_arrows", strings.gesture_arrows),
        ("gesture_ctrl_wheel", strings.gesture_ctrl_wheel),
        ("gesture_click_header", strings.gesture_click_header),
        (
            "gesture_right_click_header",
            strings.gesture_right_click_header,
        ),
        ("gesture_click_row", strings.gesture_click_row),
        ("gesture_more_menu", strings.gesture_more_menu),
        ("gesture_tab_numbers", strings.gesture_tab_numbers),
        ("gesture_middle_click", strings.gesture_middle_click),
        ("gesture_left_right", strings.gesture_left_right),
        ("gesture_up_down", strings.gesture_up_down),
        ("gesture_drop_files", strings.gesture_drop_files),
    ] {
        // The key box keeps at least 80 DIP of the row for the action.
        list.push(Check {
            key,
            text,
            room: panel - scale(80, dpi),
            font: fonts.caption(),
        });
    }
    list
}

#[test]
fn every_fixed_width_label_fits_in_every_language() {
    let mut failures = Vec::new();
    let mut widest = Vec::new();
    // SAFETY: The screen DC is released below.
    let device = unsafe { GetDC(None) };
    assert!(!device.0.is_null(), "no screen DC");
    for dpi in [96, 144, 192] {
        let Ok(resources) = ThemeResources::new(ThemeMode::Light, dpi, 100) else {
            failures.push(format!("theme resources at {dpi} DPI"));
            continue;
        };
        let layout = UiMetrics::for_dpi(dpi);
        let dialog = dialog_font(device, dpi);
        for language in Language::ALL {
            let strings = strings_for(language);
            for check in checks(strings, &resources, layout, dialog, dpi) {
                let width = text_width(device, check.font, check.text);
                if dpi == 96 {
                    widest.push(format!(
                        "{} {}: {width} of {} px",
                        language.tag(),
                        check.key,
                        check.room
                    ));
                }
                if width > check.room || check.text.is_empty() {
                    failures.push(format!(
                        "{} at {dpi} DPI: {} {:?} is {width} px, room {} px",
                        language.tag(),
                        check.key,
                        check.text,
                        check.room
                    ));
                }
            }
            // The language choices are written in their own language.
            for setting in LanguageSetting::ALL {
                let label = setting.label_in(language);
                let width = text_width(device, resources.fonts.body(), &label);
                let panel = layout.settings_panel_width
                    - (resources.metrics.space_4 + resources.metrics.space_1) * 2;
                if width > panel - resources.metrics.space_6 {
                    failures.push(format!("{} {label:?} is {width} px", language.tag()));
                }
            }
        }
        // SAFETY: The dialog font was created by `dialog_font`.
        let _ = unsafe { DeleteObject(HGDIOBJ(dialog.0.0)) };
    }
    // SAFETY: Releases the screen DC taken above.
    unsafe {
        ReleaseDC(None, device);
    }
    assert!(
        failures.is_empty(),
        "clipped labels:\n{}\nwidths at 96 DPI:\n{}",
        failures.join("\n"),
        widest.join("\n")
    );
}
