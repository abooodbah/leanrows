//! Fixed native theme resources for the `LeanRows` shell.
//!
//! The module owns one small DPI-specific GDI resource set. It never creates
//! resources per row, and high contrast always replaces branded colors with
//! live Windows system colors. Client surfaces remain deliberately opaque so
//! `LeanRows` matches `LeanMark`'s quiet, warm visual language without Mica or
//! Acrylic changing the intended palette.

use std::mem::{size_of, size_of_val};

use windows::Win32::Foundation::{COLORREF, ERROR_SUCCESS, HWND};
use windows::Win32::Graphics::Dwm::{
    DWMSBT_NONE, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DEFAULT, DWMWCP_ROUND, DWMWINDOWATTRIBUTE,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT,
    CreateFontIndirectW, CreateSolidBrush, DeleteObject, FW_NORMAL, FW_SEMIBOLD, GetSysColor,
    HBRUSH, HFONT, HGDIOBJ, LOGFONTW, SYS_COLOR_INDEX,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyW,
    RegGetValueW, RegSetValueExW,
};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::HiDpi::SystemParametersInfoForDpi;
use windows::Win32::UI::WindowsAndMessaging::{
    NONCLIENTMETRICSW, SPI_GETHIGHCONTRAST, SPI_GETNONCLIENTMETRICS,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::{Result as WindowsResult, w};

use crate::ShellError;

const BASE_DPI: u32 = 96;
const REGISTRY_THEME_BUFFER_UNITS: usize = 16;

/// User-selected preference persisted under `HKCU\\Software\\LeanRows`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    #[must_use]
    pub(super) const fn next(self) -> Self {
        match self {
            Self::System => Self::Light,
            Self::Light => Self::Dark,
            Self::Dark => Self::System,
        }
    }

    #[must_use]
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    /// Missing, malformed, or inaccessible state fails safely to System.
    #[must_use]
    pub(super) fn load() -> Self {
        load_theme_mode().unwrap_or_default()
    }

    /// Returns false on a non-fatal registry failure; it never panics.
    pub(super) fn save(self) -> bool {
        save_theme_mode(self)
    }

    const fn registry_data(self) -> &'static [u8] {
        match self {
            Self::System => b"s\0y\0s\0t\0e\0m\0\0\0",
            Self::Light => b"l\0i\0g\0h\0t\0\0\0",
            Self::Dark => b"d\0a\0r\0k\0\0\0",
        }
    }
}

/// Palette selected after system preference and high contrast are applied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EffectiveTheme {
    Light,
    Dark,
    HighContrast,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct UiColor {
    red: u8,
    green: u8,
    blue: u8,
}

impl UiColor {
    const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    #[must_use]
    pub(super) fn colorref(self) -> COLORREF {
        COLORREF(u32::from(self.red) | (u32::from(self.green) << 8) | (u32::from(self.blue) << 16))
    }

    fn from_colorref(value: u32) -> Self {
        Self::rgb(
            u8::try_from(value & 0xff).unwrap_or_default(),
            u8::try_from((value >> 8) & 0xff).unwrap_or_default(),
            u8::try_from((value >> 16) & 0xff).unwrap_or_default(),
        )
    }
}

/// LeanMark-aligned semantic colors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Palette {
    pub(super) canvas: UiColor,
    pub(super) surface: UiColor,
    pub(super) surface_muted: UiColor,
    pub(super) border: UiColor,
    pub(super) text_primary: UiColor,
    pub(super) text_secondary: UiColor,
    pub(super) text_disabled: UiColor,
    pub(super) accent: UiColor,
    pub(super) accent_text: UiColor,
    pub(super) focus: UiColor,
    pub(super) selection: UiColor,
    pub(super) selection_text: UiColor,
    pub(super) find: UiColor,
    pub(super) find_current: UiColor,
    pub(super) warning_surface: UiColor,
    pub(super) danger_surface: UiColor,
}

impl Palette {
    #[must_use]
    pub(super) fn for_theme(theme: EffectiveTheme) -> Self {
        match theme {
            EffectiveTheme::Light => light_palette(),
            EffectiveTheme::Dark => dark_palette(),
            EffectiveTheme::HighContrast => high_contrast_palette(),
        }
    }
}

/// Layout tokens scaled from 96-DPI logical pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct UiMetrics {
    pub(super) dpi: u32,
    pub(super) space_1: i32,
    pub(super) space_2: i32,
    pub(super) space_3: i32,
    pub(super) space_4: i32,
    pub(super) space_6: i32,
    pub(super) topbar_height: i32,
    pub(super) wordmark_size: i32,
    pub(super) control_target: i32,
    pub(super) document_strip_height: i32,
    pub(super) table_header_height: i32,
    pub(super) table_row_height: i32,
    pub(super) status_height: i32,
    pub(super) progress_height: i32,
    pub(super) icon_small: i32,
    pub(super) icon_command: i32,
    pub(super) border_width: i32,
    pub(super) focus_width: i32,
    pub(super) radius_small: i32,
    pub(super) radius_medium: i32,
}

impl UiMetrics {
    #[must_use]
    pub(super) fn for_dpi(dpi: u32) -> Self {
        let dpi = normalized_dpi(dpi);
        Self {
            dpi,
            space_1: scaled(4, dpi),
            space_2: scaled(8, dpi),
            space_3: scaled(12, dpi),
            space_4: scaled(16, dpi),
            space_6: scaled(24, dpi),
            topbar_height: scaled(56, dpi),
            wordmark_size: scaled(32, dpi),
            control_target: scaled(40, dpi),
            document_strip_height: scaled(32, dpi),
            table_header_height: scaled(32, dpi),
            table_row_height: scaled(28, dpi),
            status_height: scaled(28, dpi),
            progress_height: scaled(2, dpi),
            icon_small: scaled(16, dpi),
            icon_command: scaled(20, dpi),
            border_width: scaled(1, dpi).max(1),
            focus_width: scaled(2, dpi).max(1),
            radius_small: scaled(4, dpi),
            radius_medium: scaled(6, dpi),
        }
    }
}

/// Fixed GDI resources for one top-level window at one DPI.
#[derive(Debug)]
pub(super) struct ThemeResources {
    pub(super) preference: ThemeMode,
    pub(super) effective: EffectiveTheme,
    pub(super) metrics: UiMetrics,
    pub(super) palette: Palette,
    pub(super) fonts: ThemeFonts,
    pub(super) brushes: ThemeBrushes,
}

impl ThemeResources {
    pub(super) fn new(preference: ThemeMode, dpi: u32) -> Result<Self, ShellError> {
        let effective = effective_theme(preference);
        let metrics = UiMetrics::for_dpi(dpi);
        let palette = Palette::for_theme(effective);
        Ok(Self {
            preference,
            effective,
            metrics,
            palette,
            fonts: ThemeFonts::new(metrics.dpi)?,
            brushes: ThemeBrushes::new(palette)?,
        })
    }

    /// Unsupported attributes are ignored so older Windows versions retain
    /// ordinary native chrome.
    #[must_use]
    pub(super) fn apply_window_chrome(&self, window: HWND) -> DwmChromeResult {
        apply_window_chrome(window, self.effective)
    }
}

#[derive(Debug)]
pub(super) struct ThemeFonts {
    body: OwnedFont,
    semibold: OwnedFont,
    caption: OwnedFont,
    heading: OwnedFont,
}

impl ThemeFonts {
    fn new(dpi: u32) -> Result<Self, ShellError> {
        let body_definition = message_logfont(dpi);
        let mut semibold_definition = body_definition;
        semibold_definition.lfWeight = i32::try_from(FW_SEMIBOLD.0).unwrap_or(600);
        let mut caption_definition = body_definition;
        caption_definition.lfHeight = caption_height(body_definition.lfHeight);
        let mut heading_definition = semibold_definition;
        heading_definition.lfHeight = -scaled(28, normalized_dpi(dpi));
        Ok(Self {
            body: OwnedFont::new(&body_definition)?,
            semibold: OwnedFont::new(&semibold_definition)?,
            caption: OwnedFont::new(&caption_definition)?,
            heading: OwnedFont::new(&heading_definition)?,
        })
    }

    #[must_use]
    pub(super) const fn body(&self) -> HFONT {
        self.body.0
    }

    #[must_use]
    pub(super) const fn semibold(&self) -> HFONT {
        self.semibold.0
    }

    #[must_use]
    pub(super) const fn caption(&self) -> HFONT {
        self.caption.0
    }

    #[must_use]
    pub(super) const fn heading(&self) -> HFONT {
        self.heading.0
    }
}

#[derive(Debug)]
pub(super) struct ThemeBrushes {
    canvas: OwnedBrush,
    surface: OwnedBrush,
    surface_muted: OwnedBrush,
    border: OwnedBrush,
    accent: OwnedBrush,
}

impl ThemeBrushes {
    fn new(palette: Palette) -> Result<Self, ShellError> {
        Ok(Self {
            canvas: OwnedBrush::new(palette.canvas)?,
            surface: OwnedBrush::new(palette.surface)?,
            surface_muted: OwnedBrush::new(palette.surface_muted)?,
            border: OwnedBrush::new(palette.border)?,
            accent: OwnedBrush::new(palette.accent)?,
        })
    }

    #[must_use]
    pub(super) const fn canvas(&self) -> HBRUSH {
        self.canvas.0
    }

    #[must_use]
    pub(super) const fn surface(&self) -> HBRUSH {
        self.surface.0
    }

    #[must_use]
    pub(super) const fn surface_muted(&self) -> HBRUSH {
        self.surface_muted.0
    }

    #[must_use]
    pub(super) const fn border(&self) -> HBRUSH {
        self.border.0
    }

    #[must_use]
    pub(super) const fn accent(&self) -> HBRUSH {
        self.accent.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct DwmChromeResult {
    pub(super) dark_titlebar: bool,
    pub(super) rounded_corners: bool,
    pub(super) backdrop_disabled: bool,
}

fn effective_theme(preference: ThemeMode) -> EffectiveTheme {
    select_effective_theme(
        preference,
        query_high_contrast().ok(),
        query_system_uses_dark_apps(),
    )
}

fn select_effective_theme(
    preference: ThemeMode,
    high_contrast: Option<bool>,
    system_dark: Option<bool>,
) -> EffectiveTheme {
    // If the accessibility setting cannot be read, fail closed to the system
    // color palette rather than risk obscuring a contrast theme.
    if high_contrast != Some(false) {
        return EffectiveTheme::HighContrast;
    }
    match preference {
        ThemeMode::Dark => EffectiveTheme::Dark,
        ThemeMode::System if system_dark == Some(true) => EffectiveTheme::Dark,
        // A missing or malformed Windows preference safely falls back to the
        // fully supported light palette.
        ThemeMode::Light | ThemeMode::System => EffectiveTheme::Light,
    }
}

fn query_system_uses_dark_apps() -> Option<bool> {
    let mut apps_use_light_theme = 1_u32;
    let mut byte_count = u32::try_from(size_of_val(&apps_use_light_theme)).ok()?;
    // SAFETY: the registry path and value name are static; the DWORD and byte
    // count are valid writable storage for this synchronous bounded read.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut apps_use_light_theme).cast()),
            Some(&raw mut byte_count),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    parse_apps_use_light_theme(apps_use_light_theme, byte_count)
}

fn parse_apps_use_light_theme(value: u32, byte_count: u32) -> Option<bool> {
    if byte_count != u32::try_from(size_of::<u32>()).ok()? {
        return None;
    }
    match value {
        0 => Some(true),
        1 => Some(false),
        _ => None,
    }
}

fn query_high_contrast() -> WindowsResult<bool> {
    let structure_size = u32::try_from(size_of::<HIGHCONTRASTW>()).unwrap_or(u32::MAX);
    let mut contrast = HIGHCONTRASTW {
        cbSize: structure_size,
        ..Default::default()
    };
    // SAFETY: contrast is writable and advertises its exact bounded size.
    unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            structure_size,
            Some((&raw mut contrast).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS::default(),
        )?;
    }
    Ok(contrast.dwFlags.contains(HCF_HIGHCONTRASTON))
}

fn light_palette() -> Palette {
    Palette {
        canvas: UiColor::rgb(0xfa, 0xf9, 0xf7),
        surface: UiColor::rgb(0xff, 0xff, 0xff),
        surface_muted: UiColor::rgb(0xf4, 0xf2, 0xee),
        border: UiColor::rgb(0xe8, 0xe4, 0xdc),
        text_primary: UiColor::rgb(0x1a, 0x19, 0x17),
        text_secondary: UiColor::rgb(0x5a, 0x55, 0x4c),
        text_disabled: UiColor::rgb(0xa7, 0xa1, 0x95),
        accent: UiColor::rgb(0x1d, 0x4e, 0xd8),
        accent_text: UiColor::rgb(0xff, 0xff, 0xff),
        focus: UiColor::rgb(0x3b, 0x82, 0xf6),
        selection: UiColor::rgb(0xcb, 0xdc, 0xff),
        selection_text: UiColor::rgb(0x1a, 0x19, 0x17),
        find: UiColor::rgb(0xf8, 0xd6, 0x6d),
        find_current: UiColor::rgb(0xef, 0x9f, 0x27),
        warning_surface: UiColor::rgb(0xff, 0xfb, 0xeb),
        danger_surface: UiColor::rgb(0xfe, 0xf2, 0xf2),
    }
}

fn dark_palette() -> Palette {
    Palette {
        canvas: UiColor::rgb(0x0e, 0x0d, 0x0b),
        surface: UiColor::rgb(0x1a, 0x19, 0x17),
        surface_muted: UiColor::rgb(0x2b, 0x29, 0x24),
        border: UiColor::rgb(0x2b, 0x29, 0x24),
        text_primary: UiColor::rgb(0xfa, 0xf9, 0xf7),
        text_secondary: UiColor::rgb(0xd5, 0xcf, 0xc3),
        text_disabled: UiColor::rgb(0x71, 0x6b, 0x5f),
        accent: UiColor::rgb(0x1d, 0x4e, 0xd8),
        accent_text: UiColor::rgb(0xff, 0xff, 0xff),
        focus: UiColor::rgb(0x60, 0xa5, 0xfa),
        selection: UiColor::rgb(0x23, 0x46, 0x8c),
        selection_text: UiColor::rgb(0xfa, 0xf9, 0xf7),
        find: UiColor::rgb(0x73, 0x5d, 0x1e),
        find_current: UiColor::rgb(0xb4, 0x53, 0x09),
        warning_surface: UiColor::rgb(0x43, 0x35, 0x19),
        danger_surface: UiColor::rgb(0x44, 0x27, 0x26),
    }
}

fn high_contrast_palette() -> Palette {
    let window = system_color(COLOR_WINDOW);
    let text = system_color(COLOR_WINDOWTEXT);
    let highlight = system_color(COLOR_HIGHLIGHT);
    Palette {
        canvas: window,
        surface: window,
        surface_muted: window,
        border: text,
        text_primary: text,
        text_secondary: text,
        text_disabled: system_color(COLOR_GRAYTEXT),
        accent: highlight,
        accent_text: system_color(COLOR_HIGHLIGHTTEXT),
        focus: text,
        selection: highlight,
        selection_text: system_color(COLOR_HIGHLIGHTTEXT),
        find: highlight,
        find_current: highlight,
        warning_surface: window,
        danger_surface: window,
    }
}

fn system_color(index: SYS_COLOR_INDEX) -> UiColor {
    // SAFETY: index is one of the documented system-color constants above.
    UiColor::from_colorref(unsafe { GetSysColor(index) })
}

const fn normalized_dpi(dpi: u32) -> u32 {
    if dpi == 0 { BASE_DPI } else { dpi }
}

fn scaled(value: u32, dpi: u32) -> i32 {
    let scaled = u64::from(value)
        .saturating_mul(u64::from(dpi))
        .saturating_add(48)
        / 96;
    i32::try_from(scaled).unwrap_or(i32::MAX)
}

fn message_logfont(dpi: u32) -> LOGFONTW {
    let structure_size = u32::try_from(size_of::<NONCLIENTMETRICSW>()).unwrap_or(u32::MAX);
    let mut metrics = NONCLIENTMETRICSW {
        cbSize: structure_size,
        ..Default::default()
    };
    // SAFETY: metrics is writable, correctly sized, and retained for the call.
    let queried = unsafe {
        SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS.0,
            structure_size,
            Some((&raw mut metrics).cast()),
            0,
            normalized_dpi(dpi),
        )
    };
    if queried.is_ok() && metrics.lfMessageFont.lfFaceName[0] != 0 {
        metrics.lfMessageFont
    } else {
        fallback_logfont(dpi)
    }
}

fn fallback_logfont(dpi: u32) -> LOGFONTW {
    const SEGOE_UI_VARIABLE_TEXT: [u16; 23] = [
        83, 101, 103, 111, 101, 32, 85, 73, 32, 86, 97, 114, 105, 97, 98, 108, 101, 32, 84, 101,
        120, 116, 0,
    ];
    let mut definition = LOGFONTW {
        lfHeight: -scaled(12, normalized_dpi(dpi)),
        lfWeight: i32::try_from(FW_NORMAL.0).unwrap_or(400),
        ..Default::default()
    };
    definition.lfFaceName[..SEGOE_UI_VARIABLE_TEXT.len()].copy_from_slice(&SEGOE_UI_VARIABLE_TEXT);
    definition
}

fn caption_height(body_height: i32) -> i32 {
    let magnitude = body_height
        .unsigned_abs()
        .saturating_mul(11)
        .saturating_add(6)
        / 12;
    let magnitude = i32::try_from(magnitude).unwrap_or(i32::MAX).max(1);
    if body_height < 0 {
        -magnitude
    } else {
        magnitude
    }
}

fn load_theme_mode() -> Option<ThemeMode> {
    let mut buffer = [0_u16; REGISTRY_THEME_BUFFER_UNITS];
    let mut byte_count = u32::try_from(size_of_val(&buffer)).ok()?;
    // SAFETY: path/name are static; buffer and byte count are valid writable
    // storage for this synchronous bounded read.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\LeanRows"),
            w!("Theme"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&raw mut byte_count),
        )
    };
    if status != ERROR_SUCCESS || byte_count % 2 != 0 {
        return None;
    }
    let units = usize::try_from(byte_count / 2).ok()?.min(buffer.len());
    parse_theme_mode(&buffer[..units])
}

fn parse_theme_mode(value: &[u16]) -> Option<ThemeMode> {
    let end = value
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(value.len());
    let value = &value[..end];
    if matches_ascii(value, b"system") {
        Some(ThemeMode::System)
    } else if matches_ascii(value, b"light") {
        Some(ThemeMode::Light)
    } else if matches_ascii(value, b"dark") {
        Some(ThemeMode::Dark)
    } else {
        None
    }
}

fn matches_ascii(value: &[u16], expected: &[u8]) -> bool {
    value.len() == expected.len()
        && value.iter().zip(expected).all(|(left, right)| {
            u8::try_from(*left).is_ok_and(|left| left.eq_ignore_ascii_case(right))
        })
}

fn save_theme_mode(mode: ThemeMode) -> bool {
    let mut raw_key = HKEY::default();
    // SAFETY: the root and subkey are fixed; raw_key is valid writable output.
    if unsafe {
        RegCreateKeyW(
            HKEY_CURRENT_USER,
            w!("Software\\LeanRows"),
            &raw mut raw_key,
        )
    } != ERROR_SUCCESS
    {
        return false;
    }
    let key = OwnedRegistryKey(raw_key);
    // UTF-16LE including NUL; every supported Windows target is little-endian.
    let data = mode.registry_data();
    // SAFETY: key is live and data is a complete bounded REG_SZ payload.
    (unsafe { RegSetValueExW(key.0, w!("Theme"), None, REG_SZ, Some(data)) }) == ERROR_SUCCESS
}

fn apply_window_chrome(window: HWND, theme: EffectiveTheme) -> DwmChromeResult {
    let dark = i32::from(theme == EffectiveTheme::Dark);
    let corner = if theme == EffectiveTheme::HighContrast {
        DWMWCP_DEFAULT
    } else {
        DWMWCP_ROUND
    };
    DwmChromeResult {
        dark_titlebar: set_dwm_attribute(window, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark),
        rounded_corners: set_dwm_attribute(window, DWMWA_WINDOW_CORNER_PREFERENCE, &corner),
        // Explicitly clear any previous system backdrop. The call is harmless
        // on Windows versions that do not support this documented attribute.
        backdrop_disabled: set_dwm_attribute(window, DWMWA_SYSTEMBACKDROP_TYPE, &DWMSBT_NONE),
    }
}

fn set_dwm_attribute<T>(window: HWND, attribute: DWMWINDOWATTRIBUTE, value: &T) -> bool {
    let Ok(size) = u32::try_from(size_of::<T>()) else {
        return false;
    };
    // SAFETY: value matches the documented attribute payload and remains live
    // for the synchronous call. Unsupported attributes fail without mutation.
    unsafe { DwmSetWindowAttribute(window, attribute, std::ptr::from_ref(value).cast(), size) }
        .is_ok()
}

#[derive(Debug)]
struct OwnedFont(HFONT);

impl OwnedFont {
    fn new(definition: &LOGFONTW) -> Result<Self, ShellError> {
        // SAFETY: definition is initialized and retained for the call.
        let handle = unsafe { CreateFontIndirectW(std::ptr::from_ref(definition)) };
        (!handle.0.is_null())
            .then_some(Self(handle))
            .ok_or_else(|| ShellError::new("native theme font creation failed"))
    }
}

impl Drop for OwnedFont {
    fn drop(&mut self) {
        // SAFETY: this wrapper uniquely owns the font; integration must not
        // retain it in a device context beyond a paint operation.
        let _ = unsafe { DeleteObject(HGDIOBJ(self.0.0)) };
    }
}

#[derive(Debug)]
struct OwnedBrush(HBRUSH);

impl OwnedBrush {
    fn new(color: UiColor) -> Result<Self, ShellError> {
        // SAFETY: COLORREF is a value-only GDI input.
        let handle = unsafe { CreateSolidBrush(color.colorref()) };
        (!handle.0.is_null())
            .then_some(Self(handle))
            .ok_or_else(|| ShellError::new("native theme brush creation failed"))
    }
}

impl Drop for OwnedBrush {
    fn drop(&mut self) {
        // SAFETY: this wrapper uniquely owns the brush and no DC retains it.
        let _ = unsafe { DeleteObject(HGDIOBJ(self.0.0)) };
    }
}

struct OwnedRegistryKey(HKEY);

impl Drop for OwnedRegistryKey {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns one successful RegCreateKeyW result.
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EffectiveTheme, Palette, ThemeMode, UiColor, UiMetrics, parse_apps_use_light_theme,
        parse_theme_mode, select_effective_theme,
    };

    #[test]
    fn theme_cycle_and_labels_are_stable() {
        assert_eq!(ThemeMode::System.next(), ThemeMode::Light);
        assert_eq!(ThemeMode::Light.next(), ThemeMode::Dark);
        assert_eq!(ThemeMode::Dark.next(), ThemeMode::System);
        assert_eq!(ThemeMode::System.label(), "System");
        assert_eq!(ThemeMode::Light.label(), "Light");
        assert_eq!(ThemeMode::Dark.label(), "Dark");
        assert_eq!(ThemeMode::Dark.registry_data(), b"d\0a\0r\0k\0\0\0");
    }

    #[test]
    fn registry_parser_is_bounded_case_insensitive_and_fail_closed() {
        let system: Vec<u16> = "SyStEm\0".encode_utf16().collect();
        let light: Vec<u16> = "LIGHT\0".encode_utf16().collect();
        let dark: Vec<u16> = "dark\0".encode_utf16().collect();
        let invalid: Vec<u16> = "unknown\0".encode_utf16().collect();
        assert_eq!(parse_theme_mode(&system), Some(ThemeMode::System));
        assert_eq!(parse_theme_mode(&light), Some(ThemeMode::Light));
        assert_eq!(parse_theme_mode(&dark), Some(ThemeMode::Dark));
        assert_eq!(parse_theme_mode(&invalid), None);
        assert_eq!(parse_theme_mode(&[]), None);
    }

    #[test]
    fn windows_app_theme_dword_parser_is_strict_and_safe() {
        let dword_bytes = u32::try_from(size_of::<u32>()).unwrap_or_default();
        assert_eq!(parse_apps_use_light_theme(0, dword_bytes), Some(true));
        assert_eq!(parse_apps_use_light_theme(1, dword_bytes), Some(false));
        assert_eq!(parse_apps_use_light_theme(2, dword_bytes), None);
        assert_eq!(parse_apps_use_light_theme(0, dword_bytes - 1), None);
    }

    #[test]
    fn high_contrast_overrides_every_preference_and_unknown_state() {
        for preference in [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark] {
            assert_eq!(
                select_effective_theme(preference, Some(true), Some(false)),
                EffectiveTheme::HighContrast
            );
            assert_eq!(
                select_effective_theme(preference, None, Some(true)),
                EffectiveTheme::HighContrast
            );
        }
    }

    #[test]
    fn system_mode_follows_windows_and_falls_back_safely_to_light() {
        assert_eq!(
            select_effective_theme(ThemeMode::System, Some(false), Some(true)),
            EffectiveTheme::Dark
        );
        assert_eq!(
            select_effective_theme(ThemeMode::System, Some(false), Some(false)),
            EffectiveTheme::Light
        );
        assert_eq!(
            select_effective_theme(ThemeMode::System, Some(false), None),
            EffectiveTheme::Light
        );
    }

    #[test]
    fn explicit_theme_overrides_the_windows_app_preference() {
        assert_eq!(
            select_effective_theme(ThemeMode::Light, Some(false), Some(true)),
            EffectiveTheme::Light
        );
        assert_eq!(
            select_effective_theme(ThemeMode::Dark, Some(false), Some(false)),
            EffectiveTheme::Dark
        );
    }

    #[test]
    fn metrics_scale_and_zero_dpi_is_safe() {
        assert_eq!(UiMetrics::for_dpi(0), UiMetrics::for_dpi(96));
        assert_eq!(UiMetrics::for_dpi(144).topbar_height, 84);
        assert_eq!(UiMetrics::for_dpi(192).control_target, 80);
    }

    #[test]
    fn palettes_retain_leanmark_identity() {
        let light = Palette::for_theme(EffectiveTheme::Light);
        let dark = Palette::for_theme(EffectiveTheme::Dark);
        assert_eq!(light.canvas, UiColor::rgb(0xfa, 0xf9, 0xf7));
        assert_eq!(light.accent, UiColor::rgb(0x1d, 0x4e, 0xd8));
        assert_eq!(dark.canvas, UiColor::rgb(0x0e, 0x0d, 0x0b));
    }
}
