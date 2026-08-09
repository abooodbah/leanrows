//! UI-thread COM/MSAA bridge for the native owner-data list view.

#![deny(unsafe_op_in_unsafe_fn)]

use std::marker::PhantomData;
use std::mem::size_of;
use std::rc::Rc;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{COLOR_WINDOW, GetSysColorBrush};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::System::Variant::{VARIANT, VariantClear, VariantToInt32};
use windows::Win32::UI::Accessibility::{
    AccessibleObjectFromEvent, CLSID_AccPropServices, HCF_HIGHCONTRASTON, HIGHCONTRASTW,
    IAccPropServices, IAccessible, PROPID_ACC_NAME, ROLE_SYSTEM_LIST, ROLE_SYSTEM_LISTITEM,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, GCLP_HBRBACKGROUND, GetClassLongPtrW, OBJID_CLIENT, SPI_GETHIGHCONTRAST,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::{PCWSTR, w};

use crate::{
    AccessibleName, CachedRow, DisplayCell, ImmutableRowCache, ShellError, SlidingRowWindow,
};

const SMOKE_ITEM_NAME: &str = "Row 1; Preview: Accessibility smoke row";

/// Balances one successful `CoInitializeEx` call on the creating UI thread.
pub(super) struct ComApartment {
    _ui_thread_only: PhantomData<Rc<()>>,
}

impl ComApartment {
    pub(super) fn initialize() -> Result<Self, ShellError> {
        // SAFETY: COM is initialized with no reserved pointer and apartment-threaded
        // semantics before any interface is created on this UI thread.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .map_err(|error| ShellError::new(format!("COM initialization failed: {error}")))?;
        Ok(Self {
            _ui_thread_only: PhantomData,
        })
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        // SAFETY: This guard exists only after a successful `CoInitializeEx` and
        // cannot move to another thread because of its `Rc` phantom marker.
        unsafe { CoUninitialize() };
    }
}

/// Uses Windows' Dynamic Annotation API for names on realized virtual items.
pub(super) struct AccessibilityBridge {
    services: IAccPropServices,
    _ui_thread_only: PhantomData<Rc<()>>,
}

impl AccessibilityBridge {
    pub(super) fn new() -> Result<Self, ShellError> {
        // SAFETY: COM is initialized on this thread; no aggregation is requested,
        // and the returned typed interface owns its reference count.
        let services =
            unsafe { CoCreateInstance(&CLSID_AccPropServices, None, CLSCTX_INPROC_SERVER) }
                .map_err(|error| {
                    ShellError::new(format!("accessibility service creation failed: {error}"))
                })?;
        Ok(Self {
            services,
            _ui_thread_only: PhantomData,
        })
    }

    /// Supplies stable names for the top-level client and the row grid itself.
    pub(super) fn annotate_shell(&self, window: HWND, list: HWND) -> Result<(), ShellError> {
        self.set_self_name(window, w!("LeanRows"))?;
        if let Err(error) = self.set_self_name(list, w!("Rows")) {
            let _ = self.clear_self_name(window);
            return Err(error);
        }
        Ok(())
    }

    /// Clears the two persistent self-name annotations before HWND teardown.
    pub(super) fn clear_shell(&self, window: HWND, list: HWND) -> Result<(), ShellError> {
        let window_result = self.clear_self_name(window);
        let list_result = self.clear_self_name(list);
        window_result.and(list_result)
    }

    /// Annotates the intersection of immutable cached and currently realized rows.
    pub(super) fn annotate_cached_items(
        &self,
        list: HWND,
        rows: SlidingRowWindow,
        cache: &ImmutableRowCache,
    ) -> Result<(), ShellError> {
        for (absolute_row, name) in cache.accessible_entries() {
            let Some(local_row) = rows.absolute_to_local(absolute_row) else {
                continue;
            };
            let child_id = child_id_for_local_row(local_row)?;
            // SAFETY: `list` is the live list-view HWND; the object/child IDs use
            // the documented MSAA one-based child convention; `name` is immutable,
            // NUL-terminated UTF-16 retained by the cache for the entire call.
            if let Err(error) = unsafe {
                self.services.SetHwndPropStr(
                    list,
                    object_identifier(OBJID_CLIENT.0),
                    child_id,
                    PROPID_ACC_NAME,
                    PCWSTR(name.as_utf16().as_ptr()),
                )
            } {
                let _ = self.clear_cached_items(list, rows, cache);
                return Err(ShellError::new(format!(
                    "accessible item annotation failed: {error}"
                )));
            }
        }
        Ok(())
    }

    /// Clears every name that could have been applied for the supplied cache window.
    pub(super) fn clear_cached_items(
        &self,
        list: HWND,
        rows: SlidingRowWindow,
        cache: &ImmutableRowCache,
    ) -> Result<(), ShellError> {
        let mut first_error = None;
        for (absolute_row, _) in cache.accessible_entries() {
            let Some(local_row) = rows.absolute_to_local(absolute_row) else {
                continue;
            };
            let child_id = child_id_for_local_row(local_row)?;
            // SAFETY: The HWND and IDs match those used by `annotate_cached_items`;
            // the property slice remains valid for this synchronous COM call.
            if let Err(error) = unsafe {
                self.services.ClearHwndProps(
                    list,
                    object_identifier(OBJID_CLIENT.0),
                    child_id,
                    &[PROPID_ACC_NAME],
                )
            } {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), |error| {
            Err(ShellError::new(format!(
                "accessible item annotation cleanup failed: {error}"
            )))
        })
    }

    fn set_self_name(&self, window: HWND, name: PCWSTR) -> Result<(), ShellError> {
        // SAFETY: The HWND is live, the documented client/self identity is used,
        // and `name` points to immutable, statically retained UTF-16 storage.
        unsafe {
            self.services.SetHwndPropStr(
                window,
                object_identifier(OBJID_CLIENT.0),
                CHILDID_SELF,
                PROPID_ACC_NAME,
                name,
            )
        }
        .map_err(|error| {
            ShellError::new(format!("accessible self-name annotation failed: {error}"))
        })
    }

    fn clear_self_name(&self, window: HWND) -> Result<(), ShellError> {
        // SAFETY: The identity matches `set_self_name`; the property slice is
        // immutable and remains valid for this synchronous COM call.
        unsafe {
            self.services.ClearHwndProps(
                window,
                object_identifier(OBJID_CLIENT.0),
                CHILDID_SELF,
                &[PROPID_ACC_NAME],
            )
        }
        .map_err(|error| ShellError::new(format!("accessible self-name cleanup failed: {error}")))
    }
}

/// Evidence produced by the fail-closed native smoke path.
#[derive(Default)]
pub(super) struct SmokeEvidence {
    pub(super) accessibility: bool,
    pub(super) keyboard_focus: bool,
    pub(super) system_colors: bool,
}

/// Returns one immutable cached row whose expected MSAA name is known exactly.
pub(super) fn smoke_cache() -> ImmutableRowCache {
    ImmutableRowCache::new(vec![CachedRow::new_with_accessible_name(
        0,
        vec![
            DisplayCell::new("1"),
            DisplayCell::new("Accessibility smoke row"),
        ],
        AccessibleName::compose(0, &[("Preview", "Accessibility smoke row")]),
    )])
}

/// Queries the real standard-control MSAA objects and validates cleanup semantics.
pub(super) fn verify_smoke(
    bridge: &AccessibilityBridge,
    window: HWND,
    list: HWND,
    rows: SlidingRowWindow,
    cache: &ImmutableRowCache,
) -> Result<SmokeEvidence, ShellError> {
    // SAFETY: Both windows belong to the current UI thread and are visible. The
    // API's null return is ambiguous (no prior focus vs. failure), so the actual
    // postcondition is verified with `GetFocus` below.
    let _ = unsafe { SetFocus(Some(list)) };
    // SAFETY: Focus is queried on the same UI thread immediately after `SetFocus`.
    if unsafe { GetFocus() } != list {
        return Err(ShellError::new(
            "keyboard focus did not transfer to the row grid",
        ));
    }

    verify_system_color_path(window)?;

    let window_name = msaa_name(window, OBJID_CLIENT.0, CHILDID_SELF)?;
    if window_name != "LeanRows" {
        return Err(ShellError::new(format!(
            "top-level MSAA name mismatch: expected LeanRows, received {window_name:?}"
        )));
    }
    let grid_name = msaa_name(list, OBJID_CLIENT.0, CHILDID_SELF)?;
    if grid_name != "Rows" {
        return Err(ShellError::new(format!(
            "grid MSAA name mismatch: expected Rows, received {grid_name:?}"
        )));
    }
    let grid_role = msaa_role(list, OBJID_CLIENT.0, CHILDID_SELF)?;
    if grid_role != ROLE_SYSTEM_LIST {
        return Err(ShellError::new(format!(
            "grid MSAA role mismatch: expected {ROLE_SYSTEM_LIST}, received {grid_role}"
        )));
    }
    let item_name = msaa_name(list, OBJID_CLIENT.0, 1).map_err(|error| {
        ShellError::new(format!(
            "owner-data item semantics were not exposed through MSAA: {error}; a custom provider is required"
        ))
    })?;
    if item_name != SMOKE_ITEM_NAME {
        return Err(ShellError::new(format!(
            "owner-data item MSAA name mismatch: expected {SMOKE_ITEM_NAME:?}, received {item_name:?}; a custom provider is required"
        )));
    }
    let item_role = msaa_role(list, OBJID_CLIENT.0, 1).map_err(|error| {
        ShellError::new(format!(
            "owner-data item role was not exposed through MSAA: {error}; a custom provider is required"
        ))
    })?;
    if item_role != ROLE_SYSTEM_LISTITEM {
        return Err(ShellError::new(format!(
            "owner-data item MSAA role mismatch: expected {ROLE_SYSTEM_LISTITEM}, received {item_role}; a custom provider is required"
        )));
    }

    bridge.clear_cached_items(list, rows, cache)?;
    let stale_name = msaa_name(list, OBJID_CLIENT.0, 1).ok();
    let restore_result = bridge.annotate_cached_items(list, rows, cache);
    restore_result?;
    if stale_name.as_deref() == Some(SMOKE_ITEM_NAME) {
        return Err(ShellError::new(
            "cleared owner-data item retained its dynamic accessible name",
        ));
    }

    Ok(SmokeEvidence {
        accessibility: true,
        keyboard_focus: true,
        system_colors: true,
    })
}

fn verify_system_color_path(window: HWND) -> Result<(), ShellError> {
    let _high_contrast_enabled = high_contrast_enabled()?;

    // SAFETY: Both calls query process/system-owned values without transferring handles.
    let registered_brush = unsafe { GetClassLongPtrW(window, GCLP_HBRBACKGROUND) };
    // SAFETY: COLOR_WINDOW is a documented system color index.
    let system_brush = unsafe { GetSysColorBrush(COLOR_WINDOW) };
    if registered_brush == 0 || registered_brush != system_brush.0.addr() {
        return Err(ShellError::new(
            "top-level window is not using the current COLOR_WINDOW system brush",
        ));
    }
    Ok(())
}

pub(super) fn high_contrast_enabled() -> Result<bool, ShellError> {
    let structure_size = u32::try_from(size_of::<HIGHCONTRASTW>())
        .map_err(|_| ShellError::new("high-contrast structure size overflow"))?;
    let mut contrast = HIGHCONTRASTW {
        cbSize: structure_size,
        ..Default::default()
    };
    // SAFETY: `contrast` is writable and advertises its exact structure size.
    unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            structure_size,
            Some((&raw mut contrast).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS::default(),
        )
    }
    .map_err(|error| ShellError::new(format!("high-contrast query failed: {error}")))?;
    Ok(contrast.dwFlags.contains(HCF_HIGHCONTRASTON))
}

fn msaa_name(window: HWND, object_id: i32, child_id: u32) -> Result<String, ShellError> {
    let mut accessible: Option<IAccessible> = None;
    let mut child = VariantGuard::default();
    // SAFETY: Output storage is initialized and writable; the queried HWND is live.
    unsafe {
        AccessibleObjectFromEvent(
            window,
            object_identifier(object_id),
            child_id,
            &raw mut accessible,
            child.as_mut_ptr(),
        )
    }
    .map_err(|error| ShellError::new(format!("MSAA object query failed: {error}")))?;
    let accessible = accessible.ok_or_else(|| ShellError::new("MSAA returned no object"))?;
    // SAFETY: `child` was produced with the matching accessible object by
    // `AccessibleObjectFromEvent` and remains alive for the synchronous call.
    let name = unsafe { accessible.get_accName(child.as_ref()) }
        .map_err(|error| ShellError::new(format!("MSAA name query failed: {error}")))?;
    String::try_from(name)
        .map_err(|error| ShellError::new(format!("MSAA returned invalid UTF-16: {error}")))
}

fn child_id_for_local_row(local_row: i32) -> Result<u32, ShellError> {
    u32::try_from(local_row)
        .ok()
        .and_then(|row| row.checked_add(1))
        .ok_or_else(|| ShellError::new("list-view child identifier overflow"))
}

fn msaa_role(window: HWND, object_id: i32, child_id: u32) -> Result<u32, ShellError> {
    let mut accessible: Option<IAccessible> = None;
    let mut child = VariantGuard::default();
    unsafe {
        AccessibleObjectFromEvent(
            window,
            object_identifier(object_id),
            child_id,
            &raw mut accessible,
            child.as_mut_ptr(),
        )
    }
    .map_err(|error| ShellError::new(format!("MSAA object query failed: {error}")))?;
    let accessible = accessible.ok_or_else(|| ShellError::new("MSAA returned no object"))?;
    let role = VariantGuard::new(
        unsafe { accessible.get_accRole(child.as_ref()) }
            .map_err(|error| ShellError::new(format!("MSAA role query failed: {error}")))?,
    );
    let value = unsafe { VariantToInt32(role.as_ptr()) }
        .map_err(|error| ShellError::new(format!("MSAA role conversion failed: {error}")))?;
    u32::try_from(value).map_err(|_| ShellError::new("MSAA returned a negative role identifier"))
}

const fn object_identifier(value: i32) -> u32 {
    u32::from_ne_bytes(value.to_ne_bytes())
}

#[derive(Default)]
struct VariantGuard(VARIANT);

impl VariantGuard {
    const fn new(value: VARIANT) -> Self {
        Self(value)
    }

    const fn as_ptr(&self) -> *const VARIANT {
        &raw const self.0
    }

    fn as_mut_ptr(&mut self) -> *mut VARIANT {
        &raw mut self.0
    }

    const fn as_ref(&self) -> &VARIANT {
        &self.0
    }
}

impl Drop for VariantGuard {
    fn drop(&mut self) {
        // SAFETY: This guard uniquely owns the initialized VARIANT storage and
        // clears it exactly once after any MSAA-populated content is consumed.
        let _ = unsafe { VariantClear(&raw mut self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::{child_id_for_local_row, object_identifier};

    #[test]
    fn msaa_child_ids_are_one_based_and_checked() {
        assert_eq!(child_id_for_local_row(0).ok(), Some(1));
        assert_eq!(child_id_for_local_row(99_999_999).ok(), Some(100_000_000));
        assert!(child_id_for_local_row(-1).is_err());
    }

    #[test]
    fn signed_object_ids_preserve_win32_bit_patterns() {
        assert_eq!(object_identifier(-4), 0xFFFF_FFFC);
        assert_eq!(object_identifier(0), 0);
    }
}
