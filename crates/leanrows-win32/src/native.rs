#![deny(unsafe_op_in_unsafe_fn)]

mod accessibility;
mod clipboard;
mod drop_files;

use std::ffi::{OsString, c_void};
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    COLOR_WINDOW, DEFAULT_GUI_FONT, GetStockObject, GetSysColorBrush, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::{FindResourceW, GetModuleHandleW};
use windows::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, FINDMSGSTRINGW, FINDREPLACEW, FR_DIALOGTERM, FR_DOWN, FR_FINDNEXT,
    FR_HIDEWHOLEWORD, FR_MATCHCASE, FindTextW, GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST,
    OFN_HIDEREADONLY, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::Controls::{
    HDM_GETITEMCOUNT, ICC_BAR_CLASSES, ICC_LISTVIEW_CLASSES, INITCOMMONCONTROLSEX,
    InitCommonControlsEx, LIST_VIEW_ITEM_STATE_FLAGS, LVCF_SUBITEM, LVCF_TEXT, LVCF_WIDTH,
    LVCFMT_LEFT, LVCOLUMNW, LVIF_TEXT, LVIS_FOCUSED, LVIS_SELECTED, LVITEMW, LVM_DELETECOLUMN,
    LVM_ENSUREVISIBLE, LVM_GETHEADER, LVM_GETNEXTITEM, LVM_INSERTCOLUMNW, LVM_REDRAWITEMS,
    LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMCOUNT, LVM_SETITEMSTATE, LVN_GETDISPINFOW,
    LVN_ODCACHEHINT, LVNI_SELECTED, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT, LVS_OWNERDATA,
    LVS_REPORT, LVS_SHOWSELALWAYS, LVSICF_NOINVALIDATEALL, LVSICF_NOSCROLL, NMHDR, NMLVCACHEHINT,
    NMLVDISPINFOW, SB_SETTEXTW, SBARS_SIZEGRIP, STATUSCLASSNAMEW, SetWindowTheme, WC_LISTVIEWW,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateMenu, CreatePopupMenu,
    CreateWindowExW, DefWindowProcW, DestroyAcceleratorTable, DestroyMenu, DestroyWindow,
    DialogBoxParamW, DispatchMessageW, EndDialog, GCLP_HICON, GCLP_HICONSM, GWLP_USERDATA,
    GetClassLongPtrW, GetClientRect, GetDlgItemTextW, GetMessageW, GetSystemMetrics,
    GetWindowLongPtrW, HACCEL, HICON, HMENU, IDC_ARROW, IDCANCEL, IDOK, IMAGE_ICON,
    IsDialogMessageW, IsWindow, KillTimer, LR_SHARED, LoadAcceleratorsW, LoadCursorW, LoadImageW,
    MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_TASKMODAL, MF_POPUP, MF_SEPARATOR,
    MF_STRING, MSG, MessageBoxW, MoveWindow, PostMessageW, PostQuitMessage, RT_DIALOG,
    RegisterClassExW, RegisterWindowMessageW, SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON,
    SW_SHOWDEFAULT, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetDlgItemTextW,
    SetForegroundWindow, SetMenu, SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowTextW,
    ShowWindow, TranslateAcceleratorW, TranslateMessage, WINDOW_EX_STYLE, WINDOW_LONG_PTR_INDEX,
    WINDOW_STYLE, WM_APP, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_DPICHANGED, WM_DROPFILES,
    WM_GETFONT, WM_INITDIALOG, WM_NCCREATE, WM_NCDESTROY, WM_NOTIFY, WM_SETFONT, WM_SIZE, WM_TIMER,
    WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN, WS_EX_ACCEPTFILES, WS_EX_APPWINDOW, WS_EX_CLIENTEDGE,
    WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{PCWSTR, PWSTR, w};

use self::accessibility::{
    AccessibilityBridge, ComApartment, SmokeEvidence, high_contrast_enabled, smoke_cache,
    verify_smoke,
};
use crate::document_engine::{UiColumnKind, UiColumnLayout};
use crate::worker::{
    QueryDirection, Worker, WorkerEvent, WorkerPhase, WorkerQueryPhase, WorkerQueryState,
    WorkerResult,
};
use crate::{
    DocumentSmokeColumnKind, DocumentSmokeEvidence, ImmutableRowCache, ShellError, ShellOptions,
    ShellOutcome, SlidingRowWindow,
};

const LOADING_TEXT: [u16; 9] = [76, 111, 97, 100, 105, 110, 103, 8230, 0];
const EMPTY_TEXT: [u16; 1] = [0];
const RESOURCE_ICON_ID: usize = 101;
const RESOURCE_ACCELERATOR_ID: usize = 102;
const MAX_DOCUMENT_SMOKE_CELLS: usize = 4;
const MAX_STARTUP_ERROR_UTF16_UNITS: usize = 4_096;
const WM_APP_WORKER_READY: u32 = WM_APP + 1;
const DOCUMENT_SMOKE_TIMER_ID: usize = 1;
const DOCUMENT_SMOKE_TIMEOUT_MS: u32 = 10_000;
const ID_FILE_OPEN: u16 = 100;
const ID_FILE_RELOAD: u16 = 101;
const ID_FILE_EXIT: u16 = 102;
const ID_EDIT_COPY: u16 = 110;
const ID_EDIT_FIND: u16 = 111;
const ID_EDIT_FIND_NEXT: u16 = 112;
const ID_EDIT_FIND_PREVIOUS: u16 = 113;
const ID_EDIT_GOTO: u16 = 114;
const ID_HELP_ABOUT: u16 = 200;
const IDD_GOTO_ROW: usize = 201;
const IDC_GOTO_ROW_EDIT: i32 = 1001;
const MAX_COPY_ROWS: usize = 4_096;
const MAX_COPY_BYTES: usize = 1_024 * 1_024;
const MAX_COPY_UTF16_UNITS: usize = MAX_COPY_BYTES / size_of::<u16>();
const MAX_FIND_UTF16_UNITS: usize = 1_024;
#[cfg(target_pointer_width = "32")]
const DIALOG_USER_INDEX: WINDOW_LONG_PTR_INDEX = WINDOW_LONG_PTR_INDEX(8);
#[cfg(target_pointer_width = "64")]
const DIALOG_USER_INDEX: WINDOW_LONG_PTR_INDEX = WINDOW_LONG_PTR_INDEX(16);

struct AcceleratorGuard(HACCEL);

impl Drop for AcceleratorGuard {
    fn drop(&mut self) {
        // SAFETY: This guard uniquely owns the accelerator-table handle.
        let _ = unsafe { DestroyAcceleratorTable(self.0) };
    }
}

struct MenuGuard(Option<HMENU>);

impl MenuGuard {
    fn new(menu: HMENU) -> Self {
        Self(Some(menu))
    }

    fn handle(&self) -> HMENU {
        self.0.unwrap_or_default()
    }

    fn release(&mut self) {
        self.0 = None;
    }
}

impl Drop for MenuGuard {
    fn drop(&mut self) {
        if let Some(menu) = self.0.take() {
            // SAFETY: The guard owns menus until ownership transfers to a parent/window.
            let _ = unsafe { DestroyMenu(menu) };
        }
    }
}

struct WindowState {
    window: HWND,
    list: HWND,
    status: HWND,
    rows: SlidingRowWindow,
    cache: Arc<ImmutableRowCache>,
    accessibility: AccessibilityBridge,
    worker: Option<Worker>,
    current_path: Option<PathBuf>,
    active_serial: Option<u64>,
    columns: NativeColumns,
    pending_reveal: Option<u64>,
    find_message: u32,
    find_dialog: Option<FindDialogState>,
    active_find: Option<ActiveFind>,
    revealed_match: Option<RevealedMatch>,
    document_smoke: bool,
    document_smoke_observed: Arc<AtomicU64>,
    document_smoke_evidence: Arc<Mutex<Option<DocumentSmokeEvidence>>>,
    reclaimed: Arc<AtomicBool>,
}

struct GoToDialogState {
    initial_row: u64,
    selected_row: Option<u64>,
}

struct FindDialogState {
    window: HWND,
    descriptor: Box<FINDREPLACEW>,
    buffer: Box<[u16]>,
}

#[derive(Debug, Eq, PartialEq)]
struct ActiveFind {
    query_serial: u64,
    needle: Vec<u8>,
    case_sensitive: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RevealedMatch {
    query_serial: u64,
    hit_index: u64,
    absolute_row: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NativeColumns {
    kind: Option<UiColumnKind>,
    data_columns: u8,
}

impl NativeColumns {
    const fn preview() -> Self {
        Self {
            kind: Some(UiColumnKind::Preview),
            data_columns: 1,
        }
    }
}

impl WindowState {
    fn replace_cache(
        &mut self,
        rows: SlidingRowWindow,
        cache: Arc<ImmutableRowCache>,
    ) -> Result<(), ShellError> {
        self.accessibility
            .clear_cached_items(self.list, self.rows, self.cache.as_ref())?;
        self.rows = rows;
        self.cache = cache;
        self.accessibility
            .annotate_cached_items(self.list, self.rows, self.cache.as_ref())
    }

    fn clear_accessibility(&self) -> Result<(), ShellError> {
        let item_result =
            self.accessibility
                .clear_cached_items(self.list, self.rows, self.cache.as_ref());
        let shell_result = self.accessibility.clear_shell(self.window, self.list);
        item_result.and(shell_result)
    }
}

pub(crate) fn show_startup_error(message: &str) {
    let text = startup_error_text(message);
    // SAFETY: Both strings are NUL-terminated, the text allocation remains
    // alive for this modal call, and an unowned task-modal dialog is valid for
    // failures that occur before a LeanRows top-level window exists.
    let _ = unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            w!("LeanRows"),
            MB_OK | MB_ICONERROR | MB_TASKMODAL,
        )
    };
}

fn startup_error_text(message: &str) -> Vec<u16> {
    let mut text = Vec::new();
    text.extend(
        message
            .encode_utf16()
            .filter(|unit| *unit != 0)
            .take(MAX_STARTUP_ERROR_UTF16_UNITS - 1),
    );
    text.push(0);
    text
}

pub(crate) fn run(options: ShellOptions) -> Result<ShellOutcome, ShellError> {
    let shell_smoke = validate_shell_options(&options)?;
    let initial_path = options.initial_path;

    initialize_native_controls()?;
    let _com_apartment = ComApartment::initialize()?;
    let accessibility = AccessibilityBridge::new()?;

    // SAFETY: Passing `None` requests the current executable module.
    let module = unsafe { GetModuleHandleW(None) }
        .map_err(|error| ShellError::new(format!("module lookup failed: {error}")))?;
    let instance = HINSTANCE(module.0);
    register_window_class(instance)?;
    let accelerators = load_accelerators(instance)?;
    let find_message = register_find_dialog_message()?;

    let smoke_verified = Arc::new(AtomicBool::new(false));
    let document_smoke_observed = Arc::new(AtomicU64::new(0));
    let document_smoke_evidence = Arc::new(Mutex::new(None));
    let reclaimed = Arc::new(AtomicBool::new(false));
    let cache = if shell_smoke {
        Arc::new(smoke_cache())
    } else {
        Arc::new(ImmutableRowCache::default())
    };
    let state = Box::new(WindowState {
        window: HWND::default(),
        list: HWND::default(),
        status: HWND::default(),
        rows: SlidingRowWindow::new(u64::from(shell_smoke), u32::from(shell_smoke)),
        cache,
        accessibility,
        worker: None,
        current_path: None,
        active_serial: None,
        columns: NativeColumns::preview(),
        pending_reveal: None,
        find_message,
        find_dialog: None,
        active_find: None,
        revealed_match: None,
        document_smoke: options.document_smoke_test,
        document_smoke_observed: Arc::clone(&document_smoke_observed),
        document_smoke_evidence: Arc::clone(&document_smoke_evidence),
        reclaimed: Arc::clone(&reclaimed),
    });
    let (window, state_pointer) = create_shell_window(instance, state, reclaimed.as_ref())?;

    if let Err(error) = install_menu(window) {
        let _ = destroy_shell_window(window);
        return Err(error);
    }

    let smoke_evidence = if options.smoke_test {
        run_smoke(window, state_pointer, &smoke_verified)?
    } else if options.document_smoke_test {
        let evidence = match verify_shell_smoke(window, state_pointer, &smoke_verified) {
            Ok(evidence) => evidence,
            Err(error) => {
                let _ = destroy_shell_window(window);
                return Err(error);
            }
        };
        let path = initial_path.ok_or_else(|| {
            ShellError::new("document smoke input disappeared before worker submission")
        })?;
        if let Err(error) = queue_path(window, path) {
            let _ = destroy_shell_window(window);
            return Err(error);
        }
        // SAFETY: The timer belongs to this live UI-thread window and posts no pointers.
        if unsafe {
            SetTimer(
                Some(window),
                DOCUMENT_SMOKE_TIMER_ID,
                DOCUMENT_SMOKE_TIMEOUT_MS,
                None,
            )
        } == 0
        {
            let _ = destroy_shell_window(window);
            return Err(ShellError::new("document smoke timeout timer failed"));
        }
        evidence
    } else {
        if let Some(path) = initial_path
            && let Err(error) = queue_path(window, path)
        {
            let _ = destroy_shell_window(window);
            return Err(error);
        }
        show_shell_window(window);
        SmokeEvidence::default()
    };

    message_loop_with_cleanup(window, accelerators.0)?;
    let document_smoke = take_document_smoke(&document_smoke_evidence);
    if options.document_smoke_test && document_smoke.is_none() {
        return Err(ShellError::new(
            "document smoke did not observe a usable real row before shutdown",
        ));
    }
    Ok(ShellOutcome {
        smoke_controls_verified: smoke_verified.load(Ordering::Acquire),
        smoke_accessibility_verified: smoke_evidence.accessibility,
        smoke_keyboard_focus_verified: smoke_evidence.keyboard_focus,
        smoke_system_colors_verified: smoke_evidence.system_colors,
        document_smoke,
    })
}

fn take_document_smoke(
    evidence: &Mutex<Option<DocumentSmokeEvidence>>,
) -> Option<DocumentSmokeEvidence> {
    match evidence.lock() {
        Ok(mut evidence) => evidence.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    }
}

fn validate_shell_options(options: &ShellOptions) -> Result<bool, ShellError> {
    if options.smoke_test && options.document_smoke_test {
        return Err(ShellError::new("native smoke modes are mutually exclusive"));
    }
    if options.document_smoke_test && options.initial_path.is_none() {
        return Err(ShellError::new("document smoke requires one input file"));
    }
    Ok(options.smoke_test || options.document_smoke_test)
}

fn initialize_native_controls() -> Result<(), ShellError> {
    let controls_size = u32::try_from(size_of::<INITCOMMONCONTROLSEX>())
        .map_err(|_| ShellError::new("common-control structure size overflow"))?;
    let controls = INITCOMMONCONTROLSEX {
        dwSize: controls_size,
        dwICC: ICC_LISTVIEW_CLASSES | ICC_BAR_CLASSES,
    };
    // SAFETY: `controls` is initialized to the documented structure size and flags.
    if !unsafe { InitCommonControlsEx(&raw const controls) }.as_bool() {
        return Err(ShellError::new("common-control initialization failed"));
    }
    Ok(())
}

fn register_find_dialog_message() -> Result<u32, ShellError> {
    // SAFETY: FINDMSGSTRINGW is a process-lifetime, NUL-terminated system string.
    let message = unsafe { RegisterWindowMessageW(FINDMSGSTRINGW) };
    (message != 0)
        .then_some(message)
        .ok_or_else(|| ShellError::new("Find dialog message registration failed"))
}

fn create_shell_window(
    instance: HINSTANCE,
    state: Box<WindowState>,
    reclaimed: &AtomicBool,
) -> Result<(HWND, *mut WindowState), ShellError> {
    // SAFETY: DPI initialization is complete and requires no pointer arguments.
    let dpi = unsafe { GetDpiForSystem() }.max(96);
    let mut bounds = RECT {
        left: 0,
        top: 0,
        right: scale(960, dpi),
        bottom: scale(640, dpi),
    };
    // SAFETY: `bounds` is writable and the style values match the window below.
    unsafe {
        AdjustWindowRectExForDpi(
            &raw mut bounds,
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            true,
            WS_EX_APPWINDOW | WS_EX_ACCEPTFILES,
            dpi,
        )
    }
    .map_err(|error| ShellError::new(format!("window sizing failed: {error}")))?;

    // Raw ownership begins only after every pre-window fallible step succeeds.
    let state_pointer = Box::into_raw(state);

    // SAFETY: The class is registered, the state pointer remains owned by the
    // window after `WM_NCCREATE`, and all strings have static UTF-16 storage.
    let window_result = unsafe {
        CreateWindowExW(
            WS_EX_APPWINDOW | WS_EX_ACCEPTFILES,
            w!("LeanRows.NativeWindow"),
            w!("LeanRows"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            None,
            None,
            Some(instance),
            Some(state_pointer.cast_const().cast::<c_void>()),
        )
    };
    let window = match window_result {
        Ok(window) => window,
        Err(error) => {
            if !reclaimed.swap(true, Ordering::AcqRel) {
                // SAFETY: WM_NCDESTROY did not reclaim the unique Box owner.
                drop(unsafe { Box::from_raw(state_pointer) });
            }
            return Err(ShellError::new(format!("window creation failed: {error}")));
        }
    };
    Ok((window, state_pointer))
}

fn run_smoke(
    window: HWND,
    state_pointer: *mut WindowState,
    controls_verified: &AtomicBool,
) -> Result<SmokeEvidence, ShellError> {
    let verification = verify_shell_smoke(window, state_pointer, controls_verified);
    let shutdown = destroy_shell_window(window)
        .map_err(|error| ShellError::new(format!("smoke shutdown failed: {error}")));
    let evidence = verification?;
    shutdown?;
    Ok(evidence)
}

fn verify_shell_smoke(
    window: HWND,
    state_pointer: *mut WindowState,
    controls_verified: &AtomicBool,
) -> Result<SmokeEvidence, ShellError> {
    show_shell_window(window);
    // SAFETY: Window creation normally attaches this pointer through WM_NCCREATE.
    if let Some(state) = unsafe { state_pointer.as_mut() } {
        // SAFETY: Both handles were created on this UI thread and are queried read-only.
        let verified = unsafe { IsWindow(Some(state.list)) }.as_bool()
            && unsafe { IsWindow(Some(state.status)) }.as_bool();
        if verified {
            verify_native_presentation(window, state.list, state.status)?;
            controls_verified.store(true, Ordering::Release);
            let evidence = verify_smoke(
                &state.accessibility,
                window,
                state.list,
                state.rows,
                state.cache.as_ref(),
            )?;
            verify_column_lifecycle(state)?;
            verify_native_ux_primitives(state)?;
            Ok(evidence)
        } else {
            Err(ShellError::new(
                "smoke verification could not find both native child controls",
            ))
        }
    } else {
        Err(ShellError::new("smoke state was not attached"))
    }
}

fn verify_native_ux_primitives(state: &WindowState) -> Result<(), ShellError> {
    // SAFETY: The current module stays loaded for the process lifetime.
    let module = unsafe { GetModuleHandleW(None) }
        .map_err(|error| ShellError::new(format!("module lookup failed: {error}")))?;
    let dialog_name = PCWSTR(std::ptr::without_provenance::<u16>(IDD_GOTO_ROW));
    // SAFETY: This read-only lookup transfers no resource ownership.
    let dialog = unsafe { FindResourceW(Some(module), dialog_name, RT_DIALOG) };
    if dialog.0.is_null() {
        return Err(ShellError::new(
            "Go-to-row dialog resource is not embedded in the executable",
        ));
    }
    select_absolute_row(state, 0)?;
    let selected = selected_local_rows(state.list)?;
    let copied = build_copy_text(state.cache.as_ref(), state.rows, state.columns, &selected)?;
    let expected: Vec<u16> = "1\tAccessibility smoke row\0".encode_utf16().collect();
    if copied != expected {
        return Err(ShellError::new(
            "cache-only selected-row copy smoke produced unexpected text",
        ));
    }
    Ok(())
}

fn verify_column_lifecycle(state: &mut WindowState) -> Result<(), ShellError> {
    reset_data_columns(state)?;
    if header_column_count(state.list) != Some(1) {
        return Err(ShellError::new(
            "native column smoke did not reset to the Row column",
        ));
    }
    let fields = UiColumnLayout::fields(3)
        .ok_or_else(|| ShellError::new("native column smoke layout is invalid"))?;
    synchronize_data_columns(state, fields)?;
    if header_column_count(state.list) != Some(4) {
        return Err(ShellError::new(
            "native column smoke did not create three independent fields",
        ));
    }
    reset_data_columns(state)?;
    synchronize_data_columns(state, UiColumnLayout::preview())?;
    if header_column_count(state.list) != Some(2) {
        return Err(ShellError::new(
            "native column smoke did not restore Row plus Preview",
        ));
    }
    Ok(())
}

fn header_column_count(list: HWND) -> Option<i32> {
    // SAFETY: Both synchronous messages query handles/counts and carry no pointers.
    let raw_header = unsafe { SendMessageW(list, LVM_GETHEADER, None, None) }.0;
    if raw_header == 0 {
        return None;
    }
    let header = HWND(raw_header as *mut c_void);
    let count = unsafe { SendMessageW(header, HDM_GETITEMCOUNT, None, None) }.0;
    i32::try_from(count).ok().filter(|value| *value >= 0)
}

fn show_shell_window(window: HWND) {
    // SAFETY: All callers supply a live top-level HWND owned by this UI thread.
    unsafe {
        let _ = ShowWindow(window, SW_SHOWDEFAULT);
        let _ = UpdateWindow(window);
    }
}

fn load_accelerators(instance: HINSTANCE) -> Result<AcceleratorGuard, ShellError> {
    let resource = PCWSTR(std::ptr::without_provenance::<u16>(RESOURCE_ACCELERATOR_ID));
    // SAFETY: Resource 102 is compiled into this module as an ACCELERATORS table.
    // The guard releases the returned handle after the message loop exits.
    unsafe { LoadAcceleratorsW(Some(instance), resource) }
        .map(AcceleratorGuard)
        .map_err(|error| ShellError::new(format!("embedded accelerator load failed: {error}")))
}

fn install_menu(window: HWND) -> Result<(), ShellError> {
    // SAFETY: Menu handles are newly allocated and guarded until ownership transfers.
    let mut root = MenuGuard::new(unsafe { CreateMenu() }.map_err(menu_error)?);
    let mut file = MenuGuard::new(unsafe { CreatePopupMenu() }.map_err(menu_error)?);
    unsafe {
        AppendMenuW(
            file.handle(),
            MF_STRING,
            usize::from(ID_FILE_OPEN),
            w!("&Open...\tCtrl+O"),
        )
        .map_err(menu_error)?;
        AppendMenuW(
            file.handle(),
            MF_STRING,
            usize::from(ID_FILE_RELOAD),
            w!("&Reload\tF5"),
        )
        .map_err(menu_error)?;
        AppendMenuW(file.handle(), MF_SEPARATOR, 0, None).map_err(menu_error)?;
        AppendMenuW(
            file.handle(),
            MF_STRING,
            usize::from(ID_FILE_EXIT),
            w!("E&xit"),
        )
        .map_err(menu_error)?;
        AppendMenuW(
            root.handle(),
            MF_POPUP,
            file.handle().0 as usize,
            w!("&File"),
        )
        .map_err(menu_error)?;
    }
    file.release();

    let mut edit = MenuGuard::new(unsafe { CreatePopupMenu() }.map_err(menu_error)?);
    unsafe {
        append_menu_text(
            edit.handle(),
            ID_EDIT_COPY,
            w!("&Copy selected rows\tCtrl+C"),
        )?;
        AppendMenuW(edit.handle(), MF_SEPARATOR, 0, None).map_err(menu_error)?;
        append_menu_text(edit.handle(), ID_EDIT_FIND, w!("&Find...\tCtrl+F"))?;
        append_menu_text(edit.handle(), ID_EDIT_FIND_NEXT, w!("Find &next\tF3"))?;
        append_menu_text(
            edit.handle(),
            ID_EDIT_FIND_PREVIOUS,
            w!("Find &previous\tShift+F3"),
        )?;
        AppendMenuW(edit.handle(), MF_SEPARATOR, 0, None).map_err(menu_error)?;
        append_menu_text(edit.handle(), ID_EDIT_GOTO, w!("&Go to row...\tCtrl+G"))?;
        AppendMenuW(
            root.handle(),
            MF_POPUP,
            edit.handle().0 as usize,
            w!("&Edit"),
        )
        .map_err(menu_error)?;
    }
    edit.release();

    let mut help = MenuGuard::new(unsafe { CreatePopupMenu() }.map_err(menu_error)?);
    unsafe {
        AppendMenuW(
            help.handle(),
            MF_STRING,
            usize::from(ID_HELP_ABOUT),
            w!("&About LeanRows"),
        )
        .map_err(menu_error)?;
        AppendMenuW(
            root.handle(),
            MF_POPUP,
            help.handle().0 as usize,
            w!("&Help"),
        )
        .map_err(menu_error)?;
    }
    help.release();

    // SAFETY: SetMenu transfers the root and attached popup ownership to the window.
    unsafe { SetMenu(window, Some(root.handle())) }.map_err(menu_error)?;
    root.release();
    Ok(())
}

fn menu_error(error: windows::core::Error) -> ShellError {
    let message = format!("native menu setup failed: {error}");
    drop(error);
    ShellError::new(message)
}

fn append_menu_text(menu: HMENU, command: u16, text: PCWSTR) -> Result<(), ShellError> {
    // SAFETY: The caller owns the menu and supplies a live NUL-terminated label.
    unsafe { AppendMenuW(menu, MF_STRING, usize::from(command), text) }.map_err(menu_error)
}

fn handle_command(window: HWND, command: u16) {
    match command {
        ID_FILE_OPEN => match choose_file(window) {
            Ok(Some(path)) => {
                if let Err(error) = queue_path(window, path) {
                    set_status(window, &error.to_string());
                }
            }
            Ok(None) => {}
            Err(error) => set_status(window, &error.to_string()),
        },
        ID_FILE_RELOAD => {
            let pointer = state_pointer_for(window);
            // SAFETY: The path is cloned while the UI-owned state is attached.
            let path = unsafe { pointer.as_ref() }.and_then(|state| state.current_path.clone());
            if let Some(path) = path
                && let Err(error) = queue_path(window, path)
            {
                set_status(window, &error.to_string());
            }
        }
        ID_EDIT_COPY => match copy_selected_rows(window) {
            Ok((rows, bytes)) => set_status(
                window,
                &format!("Copied {rows} cached row(s) as tab-delimited text ({bytes} bytes)"),
            ),
            Err(error) => set_status(window, &format!("Copy failed: {error}")),
        },
        ID_EDIT_GOTO => {
            if let Err(error) = show_goto_row(window) {
                set_status(window, &format!("Go to row failed: {error}"));
            }
        }
        ID_EDIT_FIND => {
            if let Err(error) = show_find_dialog(window) {
                set_status(window, &format!("Find failed: {error}"));
            }
        }
        ID_EDIT_FIND_NEXT => {
            if let Err(error) = repeat_find(window, QueryDirection::Next) {
                set_status(window, &format!("Find next failed: {error}"));
            }
        }
        ID_EDIT_FIND_PREVIOUS => {
            if let Err(error) = repeat_find(window, QueryDirection::Previous) {
                set_status(window, &format!("Find previous failed: {error}"));
            }
        }
        ID_FILE_EXIT => {
            let _ = destroy_shell_window(window);
        }
        ID_HELP_ABOUT => {
            // SAFETY: Static UTF-16 strings and a valid owner are supplied.
            let _ = unsafe {
                MessageBoxW(
                    Some(window),
                    w!("LeanRows\nA native viewer for large, row-oriented files."),
                    w!("About LeanRows"),
                    MB_OK | MB_ICONINFORMATION,
                )
            };
        }
        _ => {}
    }
}

fn show_find_dialog(window: HWND) -> Result<(), ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: Commands are dispatched synchronously on the window's UI thread.
    let state = unsafe { pointer.as_mut() }
        .ok_or_else(|| ShellError::new("window state is unavailable"))?;
    if state.current_path.is_none() || state.active_serial.is_none() {
        return Err(ShellError::new("open a document before searching"));
    }
    if let Some(dialog) = state.find_dialog.as_ref()
        // SAFETY: This read-only check is for a modeless dialog on the same UI thread.
        && unsafe { IsWindow(Some(dialog.window)) }.as_bool()
    {
        // SAFETY: Bringing the existing modeless dialog forward transfers no ownership.
        let _ = unsafe { SetForegroundWindow(dialog.window) };
        return Ok(());
    }
    state.find_dialog = None;

    let mut buffer = vec![0_u16; MAX_FIND_UTF16_UNITS].into_boxed_slice();
    if let Some(active) = state.active_find.as_ref()
        && let Ok(text) = std::str::from_utf8(&active.needle)
    {
        for (destination, unit) in buffer
            .iter_mut()
            .take(MAX_FIND_UTF16_UNITS.saturating_sub(1))
            .zip(text.encode_utf16())
        {
            *destination = unit;
        }
    }
    let find_length = u16::try_from(MAX_FIND_UTF16_UNITS)
        .map_err(|_| ShellError::new("Find input limit exceeds the native dialog limit"))?;
    let mut flags = FR_DOWN | FR_HIDEWHOLEWORD;
    if state
        .active_find
        .as_ref()
        .is_some_and(|active| active.case_sensitive)
    {
        flags |= FR_MATCHCASE;
    }
    let descriptor_size = u32::try_from(size_of::<FINDREPLACEW>())
        .map_err(|_| ShellError::new("Find dialog structure size overflow"))?;
    let mut descriptor = Box::new(FINDREPLACEW {
        lStructSize: descriptor_size,
        hwndOwner: window,
        Flags: flags,
        lpstrFindWhat: PWSTR(buffer.as_mut_ptr()),
        wFindWhatLen: find_length,
        ..Default::default()
    });
    // SAFETY: Both boxed allocations remain at stable addresses until FR_DIALOGTERM.
    let dialog = unsafe { FindTextW(descriptor.as_mut()) };
    if dialog.0.is_null() {
        // SAFETY: This immediately queries the calling thread's common-dialog error.
        let code = unsafe { CommDlgExtendedError() };
        return Err(ShellError::new(format!(
            "native Find dialog failed with code 0x{:04X}",
            code.0
        )));
    }
    state.find_dialog = Some(FindDialogState {
        window: dialog,
        descriptor,
        buffer,
    });
    set_status_handle(
        state.status,
        "Find uses literal raw UTF-8 bytes; case-insensitive matching folds ASCII only",
    );
    Ok(())
}

fn repeat_find(window: HWND, direction: QueryDirection) -> Result<(), ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: Commands are dispatched synchronously on the window's UI thread.
    let state = unsafe { pointer.as_mut() }
        .ok_or_else(|| ShellError::new("window state is unavailable"))?;
    if state.active_find.is_none() {
        return show_find_dialog(window);
    }
    navigate_active_find(state, direction)
}

fn handle_find_message(state: &mut WindowState, lparam: LPARAM) {
    let Some(dialog) = state.find_dialog.as_ref() else {
        return;
    };
    let descriptor_pointer = (&raw const *dialog.descriptor).cast::<c_void>() as isize;
    if lparam.0 != descriptor_pointer {
        return;
    }
    let flags = dialog.descriptor.Flags;
    if flags.contains(FR_DIALOGTERM) {
        state.find_dialog = None;
        return;
    }
    if !flags.contains(FR_FINDNEXT) {
        return;
    }
    let case_sensitive = flags.contains(FR_MATCHCASE);
    let direction = if flags.contains(FR_DOWN) {
        QueryDirection::Next
    } else {
        QueryDirection::Previous
    };
    let needle = find_needle(&dialog.buffer);
    match needle.and_then(|needle| start_or_navigate_find(state, needle, case_sensitive, direction))
    {
        Ok(status) => set_status_handle(state.status, &status),
        Err(error) => set_status_handle(state.status, &format!("Find failed: {error}")),
    }
}

fn find_needle(buffer: &[u16]) -> Result<Vec<u8>, ShellError> {
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    if length == 0 {
        return Err(ShellError::new("enter text to find"));
    }
    let text = String::from_utf16(&buffer[..length])
        .map_err(|_| ShellError::new("Find text contains invalid Unicode"))?;
    Ok(text.into_bytes())
}

fn start_or_navigate_find(
    state: &mut WindowState,
    needle: Vec<u8>,
    case_sensitive: bool,
    direction: QueryDirection,
) -> Result<String, ShellError> {
    let is_repeat = state
        .active_find
        .as_ref()
        .is_some_and(|active| active.needle == needle && active.case_sensitive == case_sensitive);
    if is_repeat {
        navigate_active_find(state, direction)?;
        return Ok(format!(
            "Find {} requested | raw UTF-8 bytes | {}",
            query_direction_label(direction),
            query_mode_description(case_sensitive)
        ));
    }
    let document_serial = state
        .active_serial
        .ok_or_else(|| ShellError::new("open a document before searching"))?;
    let query_serial = state
        .worker
        .as_ref()
        .and_then(|worker| worker.start_query(document_serial, needle.clone(), case_sensitive))
        .ok_or_else(|| ShellError::new("the document search request was rejected"))?;
    state.active_find = Some(ActiveFind {
        query_serial,
        needle,
        case_sensitive,
    });
    state.pending_reveal = None;
    state.revealed_match = None;
    Ok(format!(
        "Find started | raw UTF-8 bytes | {}",
        query_mode_description(case_sensitive)
    ))
}

fn navigate_active_find(
    state: &mut WindowState,
    direction: QueryDirection,
) -> Result<(), ShellError> {
    let active = state
        .active_find
        .as_ref()
        .ok_or_else(|| ShellError::new("press Ctrl+F and enter text first"))?;
    let document_serial = state
        .active_serial
        .ok_or_else(|| ShellError::new("open a document before searching"))?;
    let accepted = state.worker.as_ref().is_some_and(|worker| {
        worker.navigate_query(document_serial, active.query_serial, direction)
    });
    if !accepted {
        return Err(ShellError::new("the query navigation request was rejected"));
    }
    state.pending_reveal = None;
    state.revealed_match = None;
    set_status_handle(
        state.status,
        &format!(
            "Find {} requested | raw UTF-8 bytes | {}",
            query_direction_label(direction),
            query_mode_description(active.case_sensitive)
        ),
    );
    Ok(())
}

const fn query_direction_label(direction: QueryDirection) -> &'static str {
    match direction {
        QueryDirection::Next => "next",
        QueryDirection::Previous => "previous",
    }
}

const fn query_mode_description(case_sensitive: bool) -> &'static str {
    if case_sensitive {
        "exact case"
    } else {
        "ASCII-only case-insensitive; non-ASCII bytes remain exact"
    }
}

fn copy_selected_rows(window: HWND) -> Result<(usize, usize), ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: This command runs synchronously on the UI thread.
    let state = unsafe { pointer.as_ref() }
        .ok_or_else(|| ShellError::new("window state is unavailable"))?;
    let selected = selected_local_rows(state.list)?;
    let text = build_copy_text(state.cache.as_ref(), state.rows, state.columns, &selected)?;
    let bytes = text
        .len()
        .checked_mul(size_of::<u16>())
        .ok_or_else(|| ShellError::new("copied text size overflow"))?;
    clipboard::set_unicode_text(window, &text)?;
    Ok((selected.len(), bytes))
}

fn selected_local_rows(list: HWND) -> Result<Vec<i32>, ShellError> {
    let mut selected = Vec::new();
    selected
        .try_reserve_exact(16)
        .map_err(|_| ShellError::new("selection allocation is unavailable"))?;
    let mut current = -1_i32;
    loop {
        let signed = isize::try_from(current).unwrap_or_default();
        let raw_current = usize::from_ne_bytes(signed.to_ne_bytes());
        // SAFETY: This synchronous query uses integer parameters only.
        let result = unsafe {
            SendMessageW(
                list,
                LVM_GETNEXTITEM,
                Some(WPARAM(raw_current)),
                Some(LPARAM(isize::try_from(LVNI_SELECTED).unwrap_or_default())),
            )
        };
        if result.0 == -1 {
            break;
        }
        current = i32::try_from(result.0)
            .map_err(|_| ShellError::new("selected row index exceeds the native limit"))?;
        if selected.len() == MAX_COPY_ROWS {
            return Err(ShellError::new(
                "select at most 4,096 rows for one copy operation",
            ));
        }
        selected.push(current);
    }
    if selected.is_empty() {
        return Err(ShellError::new("select one or more cached rows first"));
    }
    Ok(selected)
}

fn build_copy_text(
    cache: &ImmutableRowCache,
    rows: SlidingRowWindow,
    columns: NativeColumns,
    selected: &[i32],
) -> Result<Vec<u16>, ShellError> {
    if selected.is_empty() || selected.len() > MAX_COPY_ROWS {
        return Err(ShellError::new(
            "copy selection must contain between 1 and 4,096 rows",
        ));
    }
    let Some(_kind) = columns.kind else {
        return Err(ShellError::new("the document has no copyable data columns"));
    };
    let column_count = usize::from(columns.data_columns).saturating_add(1);
    let mut text = Vec::new();
    text.try_reserve_exact(MAX_COPY_UTF16_UNITS.min(4_096))
        .map_err(|_| ShellError::new("clipboard text allocation is unavailable"))?;
    for (row_index, local_row) in selected.iter().copied().enumerate() {
        let absolute_row = rows
            .local_to_absolute(local_row)
            .ok_or_else(|| ShellError::new("a selected row is outside the native row window"))?;
        if !cache.contains_row(absolute_row) {
            return Err(ShellError::new(
                "a selected row is not in the current cache; scroll to load it and retry",
            ));
        }
        if row_index != 0 {
            append_copy_units(&mut text, &[13, 10])?;
        }
        for column in 0..column_count {
            if column != 0 {
                append_copy_units(&mut text, &[9])?;
            }
            match cache.cell(absolute_row, column) {
                Some(cell) => {
                    let payload = cell
                        .strip_suffix(&[0])
                        .ok_or_else(|| ShellError::new("a cached cell is not NUL-terminated"))?;
                    append_copy_units(&mut text, payload)?;
                }
                None if column != 0 => {}
                None => return Err(ShellError::new("a cached row-number cell is missing")),
            }
        }
    }
    append_copy_units(&mut text, &[0])?;
    Ok(text)
}

fn append_copy_units(destination: &mut Vec<u16>, units: &[u16]) -> Result<(), ShellError> {
    let proposed = destination
        .len()
        .checked_add(units.len())
        .ok_or_else(|| ShellError::new("copied text length overflow"))?;
    if proposed > MAX_COPY_UTF16_UNITS {
        return Err(ShellError::new(
            "copied text would exceed the 1 MiB clipboard limit; select fewer rows",
        ));
    }
    destination
        .try_reserve(units.len())
        .map_err(|_| ShellError::new("clipboard text allocation is unavailable"))?;
    destination.extend_from_slice(units);
    Ok(())
}

fn show_goto_row(window: HWND) -> Result<(), ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: The state is borrowed only until the modal dialog starts.
    let initial_row = unsafe { pointer.as_ref() }
        .ok_or_else(|| ShellError::new("window state is unavailable"))?
        .rows
        .first_row();
    let mut dialog_state = GoToDialogState {
        initial_row,
        selected_row: None,
    };
    // SAFETY: The current module is live for the dialog duration.
    let module = unsafe { GetModuleHandleW(None) }
        .map_err(|error| ShellError::new(format!("module lookup failed: {error}")))?;
    let resource = PCWSTR(std::ptr::without_provenance::<u16>(IDD_GOTO_ROW));
    let parameter = LPARAM((&raw mut dialog_state).cast::<c_void>() as isize);
    // SAFETY: Resource 201 is a modal dialog and dialog_state outlives the call.
    let result = unsafe {
        DialogBoxParamW(
            Some(HINSTANCE(module.0)),
            resource,
            Some(window),
            Some(goto_dialog_proc),
            parameter,
        )
    };
    if result == -1 {
        return Err(ShellError::new(format!(
            "Go-to-row dialog failed: {}",
            windows::core::Error::from_thread()
        )));
    }
    let Some(row) = dialog_state.selected_row else {
        return Ok(());
    };
    // SAFETY: The modal call returned to the live owner on the same UI thread.
    let state = unsafe { pointer.as_mut() }
        .ok_or_else(|| ShellError::new("window state is unavailable"))?;
    begin_reveal(state, row)?;
    let phase = if state.pending_reveal.is_some() {
        "Loading"
    } else {
        "Selected"
    };
    set_status_handle(
        state.status,
        &format!("{phase} row {}", row.saturating_add(1)),
    );
    Ok(())
}

unsafe extern "system" fn goto_dialog_proc(
    dialog: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    match message {
        WM_INITDIALOG => {
            // SAFETY: lparam points to GoToDialogState for the modal call duration.
            unsafe { SetWindowLongPtrW(dialog, DIALOG_USER_INDEX, lparam.0) };
            let state = lparam.0 as *mut GoToDialogState;
            // SAFETY: The pointer was provided by show_goto_row and remains live.
            if let Some(state) = unsafe { state.as_ref() } {
                let mut value: Vec<u16> = state
                    .initial_row
                    .saturating_add(1)
                    .to_string()
                    .encode_utf16()
                    .collect();
                value.push(0);
                // SAFETY: The encoded temporary remains live for the synchronous call.
                let _ =
                    unsafe { SetDlgItemTextW(dialog, IDC_GOTO_ROW_EDIT, PCWSTR(value.as_ptr())) };
            }
            1
        }
        WM_COMMAND => {
            let command = i32::try_from(wparam.0 & 0xffff).unwrap_or_default();
            if command == IDOK.0 {
                let mut buffer = [0_u16; 32];
                // SAFETY: buffer is writable and the edit control belongs to this dialog.
                let length = unsafe { GetDlgItemTextW(dialog, IDC_GOTO_ROW_EDIT, &mut buffer) };
                let input = &buffer[..usize::try_from(length).unwrap_or_default()];
                match parse_one_based_row(input) {
                    Ok(row) => {
                        // SAFETY: DWLP_USER stores the live modal state pointer.
                        let state = unsafe { GetWindowLongPtrW(dialog, DIALOG_USER_INDEX) }
                            as *mut GoToDialogState;
                        if let Some(state) = unsafe { state.as_mut() } {
                            state.selected_row = Some(row);
                        }
                        let _ = unsafe {
                            EndDialog(dialog, isize::try_from(IDOK.0).unwrap_or_default())
                        };
                    }
                    Err(message) => {
                        let mut text: Vec<u16> = message.encode_utf16().collect();
                        text.push(0);
                        // SAFETY: The strings are NUL-terminated and the modal dialog is live.
                        let _ = unsafe {
                            MessageBoxW(
                                Some(dialog),
                                PCWSTR(text.as_ptr()),
                                w!("Invalid row number"),
                                MB_OK | MB_ICONWARNING,
                            )
                        };
                    }
                }
                1
            } else if command == IDCANCEL.0 {
                let _ =
                    unsafe { EndDialog(dialog, isize::try_from(IDCANCEL.0).unwrap_or_default()) };
                1
            } else {
                0
            }
        }
        WM_CLOSE => {
            let _ = unsafe { EndDialog(dialog, isize::try_from(IDCANCEL.0).unwrap_or_default()) };
            1
        }
        _ => 0,
    }
}

fn parse_one_based_row(input: &[u16]) -> Result<u64, &'static str> {
    if input.is_empty() {
        return Err("Enter a row number from 1 through 18,446,744,073,709,551,615.");
    }
    let mut value = 0_u64;
    for unit in input {
        let digit = match *unit {
            48..=57 => u64::from(*unit - 48),
            _ => return Err("Use decimal digits only."),
        };
        value = value
            .checked_mul(10)
            .and_then(|current| current.checked_add(digit))
            .ok_or("The row number exceeds the 64-bit document limit.")?;
    }
    value
        .checked_sub(1)
        .ok_or("Row numbers are 1-based; enter 1 or greater.")
}

fn choose_file(window: HWND) -> Result<Option<PathBuf>, ShellError> {
    let mut buffer = vec![0_u16; 32_768].into_boxed_slice();
    let dialog_size = u32::try_from(size_of::<OPENFILENAMEW>())
        .map_err(|_| ShellError::new("open-dialog structure size overflow"))?;
    let mut dialog = OPENFILENAMEW {
        lStructSize: dialog_size,
        hwndOwner: window,
        lpstrFilter: w!(
            "Data and log files\0*.csv;*.tsv;*.jsonl;*.ndjson;*.log;*.txt\0All files\0*.*\0"
        ),
        nFilterIndex: 1,
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: 32_768,
        lpstrTitle: w!("Open a large data file"),
        Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY,
        ..Default::default()
    };
    // SAFETY: The dialog owns no borrowed data after return; the path buffer is writable.
    if unsafe { GetOpenFileNameW(&raw mut dialog) }.as_bool() {
        let length = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        return Ok(Some(PathBuf::from(OsString::from_wide(&buffer[..length]))));
    }
    // SAFETY: This immediately queries the thread-local extended dialog result.
    let code = unsafe { CommDlgExtendedError() };
    if code.0 == 0 {
        Ok(None)
    } else {
        Err(ShellError::new(format!(
            "Open dialog failed with code 0x{:04X}",
            code.0
        )))
    }
}

fn set_status(window: HWND, text: &str) {
    let pointer = state_pointer_for(window);
    // SAFETY: The status handle is copied while the state remains attached.
    let Some(status) = (unsafe { pointer.as_ref() }).map(|state| state.status) else {
        return;
    };
    set_status_handle(status, text);
}

fn set_status_handle(status: HWND, text: &str) {
    let mut utf16: Vec<u16> = text.encode_utf16().filter(|unit| *unit != 0).collect();
    utf16.push(0);
    // SAFETY: SB_SETTEXTW consumes the UTF-16 data synchronously.
    unsafe {
        SendMessageW(
            status,
            SB_SETTEXTW,
            Some(WPARAM(0)),
            Some(LPARAM(utf16.as_ptr() as isize)),
        );
    }
}

fn register_window_class(instance: HINSTANCE) -> Result<(), ShellError> {
    let class_size = u32::try_from(size_of::<WNDCLASSEXW>())
        .map_err(|_| ShellError::new("window-class structure size overflow"))?;
    let large_icon = load_resource_icon(
        instance,
        // SAFETY: These calls read immutable system metrics.
        unsafe { GetSystemMetrics(SM_CXICON) },
        unsafe { GetSystemMetrics(SM_CYICON) },
    )?;
    let small_icon = load_resource_icon(
        instance,
        // SAFETY: These calls read immutable system metrics.
        unsafe { GetSystemMetrics(SM_CXSMICON) },
        unsafe { GetSystemMetrics(SM_CYSMICON) },
    )?;
    let class = WNDCLASSEXW {
        cbSize: class_size,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        // SAFETY: The system owns this cursor and brush for process lifetime.
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }
            .map_err(|error| ShellError::new(format!("cursor load failed: {error}")))?,
        hbrBackground: unsafe { GetSysColorBrush(COLOR_WINDOW) },
        hIcon: large_icon,
        hIconSm: small_icon,
        lpszClassName: w!("LeanRows.NativeWindow"),
        ..Default::default()
    };
    // SAFETY: Every pointer in `class` is null or static and the callback ABI matches.
    if unsafe { RegisterClassExW(&raw const class) } == 0 {
        return Err(ShellError::new(format!(
            "window class registration failed: {}",
            windows::core::Error::from_thread()
        )));
    }
    Ok(())
}

fn load_resource_icon(instance: HINSTANCE, width: i32, height: i32) -> Result<HICON, ShellError> {
    let resource = PCWSTR(std::ptr::without_provenance::<u16>(RESOURCE_ICON_ID));
    // SAFETY: Resource 101 is compiled into the executable; LR_SHARED leaves
    // ownership with the module/system and the requested dimensions are system metrics.
    let handle = unsafe {
        LoadImageW(
            Some(instance),
            resource,
            IMAGE_ICON,
            width,
            height,
            LR_SHARED,
        )
    }
    .map_err(|error| ShellError::new(format!("embedded application icon load failed: {error}")))?;
    Ok(HICON(handle.0))
}

fn apply_native_presentation(list: HWND, status: HWND) -> Result<(), ShellError> {
    // SAFETY: DEFAULT_GUI_FONT is a process-independent stock object that must not be deleted.
    let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    if font.0.is_null() {
        return Err(ShellError::new("system UI font is unavailable"));
    }
    // SAFETY: Both child controls consume but do not own the stock font; redraw is requested.
    unsafe {
        SendMessageW(
            list,
            WM_SETFONT,
            Some(WPARAM(font.0.addr())),
            Some(LPARAM(1)),
        );
        SendMessageW(
            status,
            WM_SETFONT,
            Some(WPARAM(font.0.addr())),
            Some(LPARAM(1)),
        );
    }
    // Explorer styling is cosmetic. Fail closed to the unthemed native control
    // whenever the high-contrast state cannot be proven off.
    if matches!(high_contrast_enabled(), Ok(false)) {
        // SAFETY: The strings are static/NUL-terminated and the call is synchronous.
        let _ = unsafe { SetWindowTheme(list, w!("Explorer"), PCWSTR::null()) };
    }
    Ok(())
}

fn verify_native_presentation(window: HWND, list: HWND, status: HWND) -> Result<(), ShellError> {
    // SAFETY: These calls query class-owned icons and child-control fonts only.
    let icons_present = unsafe { GetClassLongPtrW(window, GCLP_HICON) } != 0
        && unsafe { GetClassLongPtrW(window, GCLP_HICONSM) } != 0;
    let fonts_present = unsafe { SendMessageW(list, WM_GETFONT, None, None) }.0 != 0
        && unsafe { SendMessageW(status, WM_GETFONT, None, None) }.0 != 0;
    if !icons_present || !fonts_present {
        return Err(ShellError::new(
            "native icon or system UI font verification failed",
        ));
    }
    Ok(())
}

fn message_loop(window: HWND, accelerators: HACCEL) -> Result<(), ShellError> {
    let mut message = MSG::default();
    loop {
        // SAFETY: `message` is writable for the duration of the call.
        let result = unsafe { GetMessageW(&raw mut message, None, 0, 0) };
        if result.0 == -1 {
            return Err(ShellError::new(format!(
                "message retrieval failed: {}",
                windows::core::Error::from_thread()
            )));
        }
        if !result.as_bool() {
            return Ok(());
        }
        let find_dialog = {
            let pointer = state_pointer_for(window);
            // SAFETY: The state remains UI-thread owned until WM_NCDESTROY.
            unsafe { pointer.as_ref() }
                .and_then(|state| state.find_dialog.as_ref())
                .map(|dialog| dialog.window)
        };
        if let Some(dialog) = find_dialog {
            // SAFETY: The message and modeless dialog both belong to this UI thread.
            if unsafe { IsDialogMessageW(dialog, &raw const message) }.as_bool() {
                continue;
            }
        }
        // SAFETY: The message was populated successfully; handles are alive until WM_QUIT.
        unsafe {
            if TranslateAcceleratorW(window, accelerators, &raw const message) == 0 {
                let _ = TranslateMessage(&raw const message);
                DispatchMessageW(&raw const message);
            }
        }
    }
}

fn message_loop_with_cleanup(window: HWND, accelerators: HACCEL) -> Result<(), ShellError> {
    let loop_result = message_loop(window, accelerators);
    // SAFETY: The HWND value was created on and remains owned by this UI thread.
    if loop_result.is_err() && unsafe { IsWindow(Some(window)) }.as_bool() {
        let _ = destroy_shell_window(window);
    }
    loop_result
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message != WM_NCCREATE {
        let pointer = state_pointer_for(window);
        // SAFETY: Registered messages are handled only while the state is attached.
        if let Some(state) = unsafe { pointer.as_mut() }
            && message == state.find_message
        {
            handle_find_message(state, lparam);
            return LRESULT(0);
        }
    }
    match message {
        WM_NCCREATE => {
            // SAFETY: Win32 supplies `CREATESTRUCTW` for `WM_NCCREATE`.
            let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            // SAFETY: The pointer came from `Box::into_raw` in `run`.
            unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize) };
            LRESULT(1)
        }
        WM_CREATE => match create_children(window) {
            Ok(()) => LRESULT(0),
            Err(()) => LRESULT(-1),
        },
        WM_SIZE => {
            layout_children(window);
            LRESULT(0)
        }
        WM_DROPFILES => {
            handle_drop(window, wparam.0);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            if lparam.0 != 0 {
                // SAFETY: Win32 supplies a suggested RECT pointer for WM_DPICHANGED.
                let suggested = unsafe { &*(lparam.0 as *const RECT) };
                // SAFETY: The suggested bounds are for this top-level window.
                let _ = unsafe {
                    SetWindowPos(
                        window,
                        None,
                        suggested.left,
                        suggested.top,
                        suggested.right - suggested.left,
                        suggested.bottom - suggested.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                };
            }
            LRESULT(0)
        }
        WM_NOTIFY => {
            handle_list_notification(window, lparam);
            LRESULT(0)
        }
        WM_COMMAND => {
            if let Ok(command) = u16::try_from(wparam.0 & 0xffff) {
                handle_command(window, command);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = destroy_shell_window(window);
            LRESULT(0)
        }
        WM_APP_WORKER_READY => {
            apply_worker_event(window);
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == DOCUMENT_SMOKE_TIMER_ID => {
            // SAFETY: This timer was created for this HWND with the same identifier.
            let _ = unsafe { KillTimer(Some(window), DOCUMENT_SMOKE_TIMER_ID) };
            let pointer = state_pointer_for(window);
            // SAFETY: The state remains attached until controlled destruction completes.
            let is_document_smoke =
                unsafe { pointer.as_ref() }.is_some_and(|state| state.document_smoke);
            if is_document_smoke {
                let _ = destroy_shell_window(window);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let pointer = state_pointer_for(window);
            // SAFETY: The state remains attached through WM_NCDESTROY.
            let worker = unsafe { pointer.as_mut() }.and_then(|state| {
                close_find_dialog(state);
                let _ = state.clear_accessibility();
                state.worker.take()
            });
            drop(worker);
            // SAFETY: Standard termination for this thread's message loop.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let pointer = state_pointer_for(window);
            // SAFETY: Clearing the slot prevents reuse before reclaiming the Box.
            unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, 0) };
            if !pointer.is_null()
                // SAFETY: The state pointer is valid until this branch reclaims it.
                && !unsafe { &*pointer }.reclaimed.swap(true, Ordering::AcqRel)
            {
                // SAFETY: The raw pointer has exactly one Box owner at this point.
                drop(unsafe { Box::from_raw(pointer) });
            }
            // SAFETY: Default non-client teardown must still run.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        _ => {
            // SAFETY: Unhandled messages are delegated to the system default proc.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
    }
}

fn create_children(window: HWND) -> Result<(), ()> {
    // SAFETY: The module belongs to this process.
    let module = unsafe { GetModuleHandleW(None) }.map_err(|_| ())?;
    let instance = HINSTANCE(module.0);
    let list_style = WS_CHILD
        | WS_VISIBLE
        | WS_TABSTOP
        | WINDOW_STYLE(LVS_REPORT | LVS_OWNERDATA | LVS_SHOWSELALWAYS);
    // SAFETY: Parent, class names, styles, and module instance are valid.
    let list = unsafe {
        CreateWindowExW(
            WS_EX_CLIENTEDGE,
            WC_LISTVIEWW,
            w!("Rows"),
            list_style,
            0,
            0,
            0,
            0,
            Some(window),
            None,
            Some(instance),
            None,
        )
    }
    .map_err(|_| ())?;
    // SAFETY: Same as above; status-bar classes were initialized before window creation.
    let status = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            STATUSCLASSNAMEW,
            w!("Ready"),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(SBARS_SIZEGRIP),
            0,
            0,
            0,
            0,
            Some(window),
            None,
            Some(instance),
            None,
        )
    }
    .map_err(|_| ())?;

    let pointer = state_pointer_for(window);
    // SAFETY: `WM_NCCREATE` attached the state before `WM_CREATE`.
    let state = unsafe { pointer.as_mut() }.ok_or(())?;
    state.window = window;
    state.list = list;
    state.status = status;
    let count = state.rows.visible_rows();
    let raw_window = window.0 as usize;
    let wake_ui: Arc<dyn Fn() + Send + Sync> = Arc::new(move || post_worker_ready(raw_window));
    state.worker = Some(Worker::start(wake_ui).map_err(|_| ())?);

    insert_column(list, 0, w!("Row"), 112)?;
    insert_existing_data_columns(list, state.columns)?;
    let style = LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER;
    let style_parameter = isize::try_from(style).unwrap_or_default();
    // SAFETY: List-view messages are synchronous and use integer payloads only.
    unsafe {
        SendMessageW(
            list,
            LVM_SETEXTENDEDLISTVIEWSTYLE,
            Some(WPARAM(style as usize)),
            Some(LPARAM(style_parameter)),
        );
        SendMessageW(
            list,
            LVM_SETITEMCOUNT,
            Some(WPARAM(count as usize)),
            Some(LPARAM(0)),
        );
        SendMessageW(
            status,
            SB_SETTEXTW,
            Some(WPARAM(0)),
            Some(LPARAM(w!("Ready").as_ptr() as isize)),
        );
    }
    apply_native_presentation(list, status).map_err(|_| ())?;
    state
        .accessibility
        .annotate_shell(window, list)
        .map_err(|_| ())?;
    if state
        .accessibility
        .annotate_cached_items(list, state.rows, state.cache.as_ref())
        .is_err()
    {
        let _ = state.clear_accessibility();
        return Err(());
    }
    layout_children(window);
    Ok(())
}

fn insert_column(
    list: HWND,
    index: usize,
    title: windows::core::PCWSTR,
    width: i32,
) -> Result<(), ()> {
    let column = LVCOLUMNW {
        mask: LVCF_TEXT | LVCF_WIDTH | LVCF_SUBITEM,
        fmt: LVCFMT_LEFT,
        cx: width,
        pszText: PWSTR(title.as_ptr().cast_mut()),
        iSubItem: i32::try_from(index).map_err(|_| ())?,
        ..Default::default()
    };
    // SAFETY: The column pointer remains valid for this synchronous message.
    let result = unsafe {
        SendMessageW(
            list,
            LVM_INSERTCOLUMNW,
            Some(WPARAM(index)),
            Some(LPARAM((&raw const column).cast::<c_void>() as isize)),
        )
    };
    (result.0 != -1).then_some(()).ok_or(())
}

fn insert_existing_data_columns(list: HWND, columns: NativeColumns) -> Result<(), ()> {
    let Some(kind) = columns.kind else {
        return (columns.data_columns == 0).then_some(()).ok_or(());
    };
    for data_index in 1..=columns.data_columns {
        insert_data_column(list, kind, data_index)?;
    }
    Ok(())
}

fn insert_data_column(list: HWND, kind: UiColumnKind, data_index: u8) -> Result<(), ()> {
    let index = usize::from(data_index);
    match kind {
        UiColumnKind::Preview if data_index == 1 => insert_column(list, index, w!("Preview"), 720),
        UiColumnKind::Preview => Err(()),
        UiColumnKind::Fields => {
            let title = format!("Column {data_index}");
            let mut encoded: Vec<u16> = title.encode_utf16().collect();
            encoded.push(0);
            insert_column(list, index, PCWSTR(encoded.as_ptr()), 220)
        }
    }
}

fn reset_data_columns(state: &mut WindowState) -> Result<(), ShellError> {
    while state.columns.data_columns != 0 {
        // Deleting subitem one repeatedly is deterministic because later data
        // columns shift left; user-resized widths cannot leak into a new file.
        // SAFETY: The list is live on this UI thread and the message has no pointers.
        let result = unsafe {
            SendMessageW(
                state.list,
                LVM_DELETECOLUMN,
                Some(WPARAM(1)),
                Some(LPARAM(0)),
            )
        };
        if result.0 == 0 {
            return Err(ShellError::new("native data-column reset failed"));
        }
        state.columns.data_columns -= 1;
    }
    state.columns.kind = None;
    Ok(())
}

fn synchronize_data_columns(
    state: &mut WindowState,
    layout: UiColumnLayout,
) -> Result<(), ShellError> {
    if !layout.is_valid() {
        return Err(ShellError::new("worker supplied invalid column metadata"));
    }
    match state.columns.kind {
        Some(kind) if kind != layout.kind() => {
            return Err(ShellError::new(
                "worker changed the column kind within one document",
            ));
        }
        None => state.columns.kind = Some(layout.kind()),
        Some(_) => {}
    }
    while state.columns.data_columns < layout.data_columns() {
        let next = state.columns.data_columns.saturating_add(1);
        insert_data_column(state.list, layout.kind(), next)
            .map_err(|()| ShellError::new("native data-column creation failed"))?;
        state.columns.data_columns = next;
    }
    Ok(())
}

fn layout_children(window: HWND) {
    let pointer = state_pointer_for(window);
    // SAFETY: The state is valid for normal messages between create and destroy.
    let Some(state) = (unsafe { pointer.as_ref() }) else {
        return;
    };
    let (list, status) = (state.list, state.status);
    if list.0.is_null() || status.0.is_null() {
        return;
    }
    let mut client = RECT::default();
    // SAFETY: All HWNDs belong to this window and both RECTs are writable.
    unsafe {
        let _ = SendMessageW(status, WM_SIZE, None, None);
        if GetClientRect(window, &raw mut client).is_err() {
            return;
        }
        let mut status_rect = RECT::default();
        if windows::Win32::UI::WindowsAndMessaging::GetWindowRect(status, &raw mut status_rect)
            .is_err()
        {
            return;
        }
        let status_height = (status_rect.bottom - status_rect.top).max(0);
        let _ = MoveWindow(
            list,
            0,
            0,
            (client.right - client.left).max(0),
            (client.bottom - client.top - status_height).max(0),
            true,
        );
    }
}

fn handle_list_notification(window: HWND, lparam: LPARAM) {
    if lparam.0 == 0 {
        return;
    }
    let pointer = state_pointer_for(window);
    // SAFETY: State lifetime is tied to the window, and every WM_NOTIFY payload
    // starts with a readable NMHDR supplied synchronously by Win32.
    let (Some(state), header) = (unsafe { pointer.as_ref() }, unsafe {
        &*(lparam.0 as *const NMHDR)
    }) else {
        return;
    };
    if header.hwndFrom != state.list {
        return;
    }
    match header.code {
        LVN_GETDISPINFOW => {
            // SAFETY: The notification code identifies this exact payload type.
            let notification = unsafe { &mut *(lparam.0 as *mut NMLVDISPINFOW) };
            serve_display_text(state, notification);
        }
        LVN_ODCACHEHINT => {
            // SAFETY: The notification code identifies this exact payload type.
            let notification = unsafe { &*(lparam.0 as *const NMLVCACHEHINT) };
            request_viewport_for_hint(state, notification);
        }
        _ => {}
    }
}

fn serve_display_text(state: &WindowState, notification: &mut NMLVDISPINFOW) {
    if !notification.item.mask.contains(LVIF_TEXT) {
        return;
    }
    let Some(absolute_row) = state.rows.local_to_absolute(notification.item.iItem) else {
        return;
    };
    let subitem = usize::try_from(notification.item.iSubItem).ok();
    let cached = subitem.and_then(|column| state.cache.cell(absolute_row, column));
    let text = match cached {
        Some(text) => text,
        None if state.cache.contains_row(absolute_row) => &EMPTY_TEXT,
        None => &LOADING_TEXT,
    };
    // No allocation, parsing, locking, or I/O occurs here. The pointer targets
    // immutable cache storage or the static loading string. Smoke attestation is
    // one bounded comparison plus an atomic store.
    notification.item.pszText = PWSTR(text.as_ptr().cast_mut());
    if state.document_smoke
        && state.active_serial.is_some()
        && absolute_row == 0
        && cached.is_some()
        && subitem.is_some_and(|column| (1..=64).contains(&column))
    {
        let bit = subitem
            .and_then(|column| column.checked_sub(1))
            .and_then(|index| u32::try_from(index).ok())
            .map_or(0, |index| 1_u64.checked_shl(index).unwrap_or(0));
        state
            .document_smoke_observed
            .fetch_or(bit, Ordering::AcqRel);
    }
}

fn request_viewport_for_hint(state: &WindowState, notification: &NMLVCACHEHINT) {
    let Some(first_row) = state.rows.local_to_absolute(notification.iFrom) else {
        return;
    };
    let Some(last_row) = state.rows.local_to_absolute(notification.iTo) else {
        return;
    };
    if state.cache.cell(first_row, 0).is_some() && state.cache.cell(last_row, 0).is_some() {
        return;
    }
    if let (Some(worker), Some(serial)) = (state.worker.as_ref(), state.active_serial) {
        let _ = worker.request_viewport(serial, first_row);
    }
}

fn post_worker_ready(raw_window: usize) {
    // SAFETY: The raw value was captured from this process's HWND. Posting a
    // pointer-free wake message is valid even if shutdown has begun; failure is ignored.
    let window = HWND(raw_window as *mut c_void);
    let _ = unsafe { PostMessageW(Some(window), WM_APP_WORKER_READY, WPARAM(0), LPARAM(0)) };
}

fn queue_path(window: HWND, path: PathBuf) -> Result<(), ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: Commands are handled only while the window state is attached.
    let Some(state) = (unsafe { pointer.as_mut() }) else {
        return Err(ShellError::new("window state is unavailable"));
    };
    set_document_title(window, &path)?;
    state.replace_cache(
        SlidingRowWindow::new(0, 0),
        Arc::new(ImmutableRowCache::default()),
    )?;
    reset_data_columns(state)?;
    state.document_smoke_observed.store(0, Ordering::Release);
    state.pending_reveal = None;
    state.active_find = None;
    state.revealed_match = None;
    let worker = state
        .worker
        .as_ref()
        .ok_or_else(|| ShellError::new("document worker is unavailable"))?;
    let serial = worker.submit_path(path.clone());
    state.current_path = Some(path);
    state.active_serial = Some(serial);
    let (list, status) = (state.list, state.status);

    // SAFETY: Both messages are synchronous and carry only bounded integer/static data.
    unsafe {
        SendMessageW(list, LVM_SETITEMCOUNT, Some(WPARAM(0)), Some(LPARAM(0)));
        SendMessageW(
            status,
            SB_SETTEXTW,
            Some(WPARAM(0)),
            Some(LPARAM(w!("Opening file...").as_ptr() as isize)),
        );
    }
    Ok(())
}

fn handle_drop(window: HWND, raw_drop: usize) {
    let result = drop_files::one_path(raw_drop).and_then(|path| {
        if !is_supported_document_path(&path) {
            return Err(ShellError::new(
                "unsupported drop; use CSV, TSV, JSONL, NDJSON, LOG, or TXT",
            ));
        }
        queue_path(window, path)
    });
    if let Err(error) = result {
        set_status(window, &format!("Could not open dropped file: {error}"));
    }
}

fn is_supported_document_path(path: &Path) -> bool {
    path.extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "csv" | "tsv" | "jsonl" | "ndjson" | "log" | "txt"
            )
        })
}

fn set_document_title(window: HWND, path: &Path) -> Result<(), ShellError> {
    let title = document_title(path)?;
    // SAFETY: title is one NUL-terminated UTF-16 value for this live HWND.
    unsafe { SetWindowTextW(window, PCWSTR(title.as_ptr())) }
        .map_err(|error| ShellError::new(format!("window title update failed: {error}")))
}

fn document_title(path: &Path) -> Result<Vec<u16>, ShellError> {
    const SUFFIX: &[u16] = &[32, 0x2014, 32, 76, 101, 97, 110, 82, 111, 119, 115, 0];

    let name = path
        .file_name()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ShellError::new("document path has no file name"))?;
    let name_units = name.encode_wide().filter(|unit| *unit != 0);
    let mut title = Vec::new();
    title
        .try_reserve_exact(name_units.clone().count().saturating_add(SUFFIX.len()))
        .map_err(|_| ShellError::new("window-title allocation is unavailable"))?;
    title.extend(name_units);
    title.extend_from_slice(SUFFIX);
    Ok(title)
}

fn apply_worker_event(window: HWND) {
    let pointer = state_pointer_for(window);
    // SAFETY: The UI thread owns this state and is the only event consumer.
    let event = unsafe { pointer.as_ref() }
        .and_then(|state| state.worker.as_ref())
        .and_then(Worker::poll);
    let Some(event) = event else {
        return;
    };
    if apply_document_event(window, &event) {
        // SAFETY: The document smoke timer, if present, belongs to this HWND.
        let _ = unsafe { KillTimer(Some(window), DOCUMENT_SMOKE_TIMER_ID) };
        let _ = destroy_shell_window(window);
    }
}

fn apply_document_event(window: HWND, event: &WorkerEvent) -> bool {
    let pointer = state_pointer_for(window);
    // SAFETY: The UI thread serializes cache swaps and display notifications.
    let Some(state) = (unsafe { pointer.as_mut() }) else {
        return false;
    };
    if state.current_path.as_ref() != Some(&event.path) || state.active_serial != Some(event.serial)
    {
        return false;
    }

    if let Some(columns) = event.columns
        && let Err(error) = synchronize_data_columns(state, columns)
    {
        set_status_handle(state.status, &format!("Column update failed: {error}"));
        return state.document_smoke;
    }

    let status_text = worker_status(event, state.active_find.as_ref());
    let mut rows = SlidingRowWindow::new(event.progress.available_rows, u32::MAX);
    rows.seek(state.rows.first_row());
    if let Err(error) = state.replace_cache(rows, Arc::clone(&event.cache)) {
        set_status_handle(
            state.status,
            &format!("Accessibility cache update failed: {error}"),
        );
        return state.document_smoke;
    }
    set_native_item_count(state.list, state.rows.visible_rows());
    redraw_cached_rows(
        state.list,
        state.rows,
        event.cache_first_row,
        event.cached_rows,
    );
    if let Some(reveal) = query_match_to_reveal(event, state.active_find.as_ref())
        && state.revealed_match != Some(reveal)
    {
        state.revealed_match = Some(reveal);
        if let Err(error) = begin_reveal(state, reveal.absolute_row) {
            set_status_handle(
                state.status,
                &format!("Find match could not be displayed: {error}"),
            );
            return state.document_smoke;
        }
    }
    if let Err(error) = complete_pending_reveal(state) {
        set_status_handle(state.status, &format!("Row selection failed: {error}"));
        return state.document_smoke;
    }
    set_status_handle(state.status, &status_text);

    if !state.document_smoke {
        return false;
    }
    if !document_event_is_usable(event) {
        return true;
    }
    // SAFETY: The visible list is invalidated above; UpdateWindow synchronously
    // drives its real owner-data paint/notification path on this UI thread.
    let _ = unsafe { UpdateWindow(state.list) };
    let Some(required_mask) = required_document_smoke_mask(event) else {
        return true;
    };
    if state.document_smoke_observed.load(Ordering::Acquire) & required_mask != required_mask {
        return true;
    }
    let Some(evidence) = document_smoke_evidence(event) else {
        return true;
    };
    match state.document_smoke_evidence.lock() {
        Ok(mut slot) => *slot = Some(evidence),
        Err(poisoned) => *poisoned.into_inner() = Some(evidence),
    }
    true
}

fn begin_reveal(state: &mut WindowState, absolute_row: u64) -> Result<(), ShellError> {
    let total_rows = state.rows.total_rows();
    if total_rows == 0 {
        return Err(ShellError::new("no indexed rows are available yet"));
    }
    if absolute_row >= total_rows {
        return Err(ShellError::new(format!(
            "row {} is not indexed; the current available range is 1 through {total_rows}",
            absolute_row.saturating_add(1)
        )));
    }
    let serial = state
        .active_serial
        .ok_or_else(|| ShellError::new("open a document before navigating"))?;
    let mut rows = state.rows;
    rows.center_on(absolute_row);
    state.replace_cache(rows, Arc::clone(&state.cache))?;
    set_native_item_count(state.list, state.rows.visible_rows());
    state.pending_reveal = Some(absolute_row);
    let requested = state
        .worker
        .as_ref()
        .is_some_and(|worker| worker.request_viewport(serial, absolute_row));
    if !requested {
        state.pending_reveal = None;
        return Err(ShellError::new(
            "the document viewport request was rejected",
        ));
    }
    complete_pending_reveal(state)
}

fn complete_pending_reveal(state: &mut WindowState) -> Result<(), ShellError> {
    let Some(absolute_row) = state.pending_reveal else {
        return Ok(());
    };
    if !state.cache.contains_row(absolute_row) {
        return Ok(());
    }
    select_absolute_row(state, absolute_row)?;
    state.pending_reveal = None;
    Ok(())
}

fn query_match_to_reveal(
    event: &WorkerEvent,
    active: Option<&ActiveFind>,
) -> Option<RevealedMatch> {
    let query = event.query.as_ref()?;
    if active.is_none_or(|active| active.query_serial != query.query_serial) {
        return None;
    }
    if query.phase != WorkerQueryPhase::MatchReady {
        return None;
    }
    Some(RevealedMatch {
        query_serial: query.query_serial,
        hit_index: query.active_hit_index?,
        absolute_row: query.active_row?,
    })
}

fn select_absolute_row(state: &WindowState, absolute_row: u64) -> Result<(), ShellError> {
    let local_row = state
        .rows
        .absolute_to_local(absolute_row)
        .ok_or_else(|| ShellError::new("target row is outside the current native row window"))?;
    let selection_state = LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0);
    let mut clear = LVITEMW {
        stateMask: selection_state,
        ..Default::default()
    };
    // SAFETY: -1 applies the supplied state mask to every virtual item; the
    // pointer remains valid through this synchronous call.
    let cleared = unsafe {
        SendMessageW(
            state.list,
            LVM_SETITEMSTATE,
            Some(WPARAM(usize::MAX)),
            Some(LPARAM((&raw mut clear).cast::<c_void>() as isize)),
        )
    };
    if cleared.0 == 0 {
        return Err(ShellError::new(
            "existing list selection could not be cleared",
        ));
    }
    let mut selected = LVITEMW {
        state: selection_state,
        stateMask: selection_state,
        ..Default::default()
    };
    let local_parameter = usize::try_from(local_row)
        .map_err(|_| ShellError::new("target row exceeds the native index limit"))?;
    // SAFETY: local_row is mapped within the current virtual item count.
    let applied = unsafe {
        SendMessageW(
            state.list,
            LVM_SETITEMSTATE,
            Some(WPARAM(local_parameter)),
            Some(LPARAM((&raw mut selected).cast::<c_void>() as isize)),
        )
    };
    if applied.0 == 0 {
        return Err(ShellError::new("target row could not be selected"));
    }
    // SAFETY: The index is valid and focus stays on this UI thread.
    unsafe {
        SendMessageW(
            state.list,
            LVM_ENSUREVISIBLE,
            Some(WPARAM(local_parameter)),
            Some(LPARAM(0)),
        );
        let _ = SetFocus(Some(state.list));
    }
    Ok(())
}

fn set_native_item_count(list: HWND, visible_rows: u32) {
    let item_count_flags =
        isize::try_from(LVSICF_NOINVALIDATEALL | LVSICF_NOSCROLL).unwrap_or_default();
    // SAFETY: The count is capped by SlidingRowWindow and carries no pointer.
    unsafe {
        SendMessageW(
            list,
            LVM_SETITEMCOUNT,
            Some(WPARAM(visible_rows as usize)),
            Some(LPARAM(item_count_flags)),
        );
    }
}

fn document_event_is_usable(event: &WorkerEvent) -> bool {
    let WorkerResult::Ready { size } = &event.result else {
        return false;
    };
    let Some(columns) = event.columns else {
        return false;
    };
    let retained_data_cells = event
        .cache
        .cell_count(0)
        .unwrap_or_default()
        .saturating_sub(1);
    *size == event.progress.source_bytes
        && event.cache_first_row == 0
        && event.cached_rows > 0
        && event.progress.available_rows > 0
        && event.cache.cell(0, 0).is_some()
        && event.cache.cell(0, 1).is_some()
        && columns.is_valid()
        && retained_data_cells > 0
        && retained_data_cells <= usize::from(columns.data_columns())
        && event.phase != WorkerPhase::Failed
}

fn required_document_smoke_mask(event: &WorkerEvent) -> Option<u64> {
    if !document_event_is_usable(event) {
        return None;
    }
    let retained = event.cache.cell_count(0)?.saturating_sub(1).min(2);
    let shift = u32::try_from(retained).ok()?;
    1_u64.checked_shl(shift).map(|exclusive| exclusive - 1)
}

fn document_smoke_evidence(event: &WorkerEvent) -> Option<DocumentSmokeEvidence> {
    let WorkerResult::Ready { size } = &event.result else {
        return None;
    };
    let columns = event.columns?;
    let retained = event.cache.cell_count(0)?.saturating_sub(1);
    let sample_count = retained.min(MAX_DOCUMENT_SMOKE_CELLS);
    let mut sample_cells = Vec::new();
    sample_cells.try_reserve_exact(sample_count).ok()?;
    for column in 1..=sample_count {
        sample_cells.push(decode_cache_cell(event.cache.as_ref(), 0, column)?);
    }
    Some(DocumentSmokeEvidence {
        source_bytes: *size,
        cache_first_row: event.cache_first_row,
        cached_rows: event.cached_rows,
        first_row: decode_cache_cell(event.cache.as_ref(), 0, 0)?,
        preview: decode_cache_cell(event.cache.as_ref(), 0, 1)?,
        column_kind: match columns.kind() {
            UiColumnKind::Preview => DocumentSmokeColumnKind::Preview,
            UiColumnKind::Fields => DocumentSmokeColumnKind::Fields,
        },
        data_columns: columns.data_columns(),
        sample_cells,
        scanned_bytes: event.progress.scanned_bytes,
        indexed_rows: event.progress.indexed_rows,
        available_rows: event.progress.available_rows,
        scan_complete: event.progress.complete,
        phase: match event.phase {
            WorkerPhase::ViewportReady => crate::DocumentSmokePhase::ViewportReady,
            WorkerPhase::Scanning => crate::DocumentSmokePhase::Scanning,
            WorkerPhase::Complete => crate::DocumentSmokePhase::Complete,
            WorkerPhase::Failed => return None,
        },
    })
}

fn decode_cache_cell(cache: &ImmutableRowCache, row: u64, column: usize) -> Option<String> {
    let cell = cache.cell(row, column)?;
    let payload = cell.strip_suffix(&[0])?;
    String::from_utf16(payload).ok()
}

fn worker_status(event: &WorkerEvent, active_find: Option<&ActiveFind>) -> String {
    use std::fmt::Write as _;

    if let WorkerResult::Failed { message } = &event.result {
        let reload = if failure_requires_reload(message) {
            " | File changed: press F5 or choose File > Reload; automatic reload is disabled"
        } else {
            ""
        };
        return format!("Could not read file: {message}{reload}");
    }
    if let Some(query) = event.query.as_ref()
        && let Some(active) = active_find
        && active.query_serial == query.query_serial
    {
        return query_status(query, active.case_sensitive);
    }
    let progress = event.progress;
    let percent = progress_percent(progress.scanned_bytes, progress.source_bytes);
    let phase = match event.phase {
        WorkerPhase::ViewportReady => "Rows ready; indexing",
        WorkerPhase::Scanning => "Indexing",
        WorkerPhase::Complete => "Ready",
        WorkerPhase::Failed => "Failed",
    };
    let mut status = format!(
        "{phase} | {} rows | {} / {} bytes | {percent}%",
        progress.available_rows, progress.scanned_bytes, progress.source_bytes
    );
    if event.source_truncated_rows > 0
        || event.display_truncated_rows > 0
        || event.escaped_non_utf8_rows > 0
    {
        let _ = write!(
            status,
            " | warnings: {} source-truncated, {} display-truncated, {} escaped",
            event.source_truncated_rows, event.display_truncated_rows, event.escaped_non_utf8_rows
        );
    }
    status
}

fn query_status(query: &WorkerQueryState, case_sensitive: bool) -> String {
    let phase = match query.phase {
        WorkerQueryPhase::Searching => "searching",
        WorkerQueryPhase::MatchReady => "match ready",
        WorkerQueryPhase::Complete => "complete",
        WorkerQueryPhase::QuotaExceeded => "hit-cache limit reached",
        WorkerQueryPhase::Failed => "failed",
    };
    let percent = progress_percent(query.scanned_bytes, query.source_bytes);
    let active = match (query.active_hit_index, query.active_row) {
        (Some(hit), Some(row)) => format!(
            " | match {} at row {}",
            hit.saturating_add(1),
            row.saturating_add(1)
        ),
        _ => String::new(),
    };
    let reload = if failure_requires_reload(&query.message) {
        " | File changed: press F5 or choose File > Reload; automatic reload is disabled"
    } else {
        ""
    };
    format!(
        "Find: {phase} | {} | {} / {} bytes | {percent}% | {} stored hit(s){active} | raw UTF-8 bytes | {}{reload}",
        query.message,
        query.scanned_bytes,
        query.source_bytes,
        query.stored_hits,
        query_mode_description(case_sensitive)
    )
}

fn progress_percent(scanned_bytes: u64, source_bytes: u64) -> u64 {
    if source_bytes == 0 {
        return 100;
    }
    u64::try_from(u128::from(scanned_bytes).saturating_mul(100) / u128::from(source_bytes))
        .unwrap_or(100)
        .min(100)
}

fn failure_requires_reload(message: &str) -> bool {
    let folded = message.to_ascii_lowercase();
    folded.contains("source changed")
        || folded.contains("snapshot is no longer trustworthy")
        || folded.contains("open document changed")
}

fn redraw_cached_rows(list: HWND, rows: SlidingRowWindow, cache_first_row: u64, cached_rows: u32) {
    if cached_rows == 0 || rows.visible_rows() == 0 {
        return;
    }
    let cache_last = cache_first_row.saturating_add(u64::from(cached_rows).saturating_sub(1));
    let visible_last = rows
        .first_row()
        .saturating_add(u64::from(rows.visible_rows()).saturating_sub(1));
    let first = cache_first_row.max(rows.first_row());
    let end_row = cache_last.min(visible_last);
    if first > end_row {
        return;
    }
    let (Some(local_first), Some(local_last)) = (
        rows.absolute_to_local(first),
        rows.absolute_to_local(end_row),
    ) else {
        return;
    };
    let (Ok(first_parameter), Ok(last_parameter)) =
        (usize::try_from(local_first), isize::try_from(local_last))
    else {
        return;
    };
    // SAFETY: Both local indices are checked intersections within the current
    // bounded native item count and the call consumes no pointers.
    unsafe {
        SendMessageW(
            list,
            LVM_REDRAWITEMS,
            Some(WPARAM(first_parameter)),
            Some(LPARAM(last_parameter)),
        );
    }
}

fn state_pointer_for(window: HWND) -> *mut WindowState {
    // SAFETY: Reading the application-owned pointer slot is always permitted.
    unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) as *mut WindowState }
}

fn close_find_dialog(state: &mut WindowState) {
    let dialog = state.find_dialog.as_ref().map(|dialog| dialog.window);
    if let Some(dialog) = dialog
        // SAFETY: This check and destruction run on the dialog's owning UI thread.
        && unsafe { IsWindow(Some(dialog)) }.as_bool()
    {
        // FR_DIALOGTERM may synchronously clear state.find_dialog; its boxed
        // descriptor and buffer stay alive throughout DestroyWindow.
        let _ = unsafe { DestroyWindow(dialog) };
    }
    state.find_dialog = None;
}

fn destroy_shell_window(window: HWND) -> Result<(), ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: The pointer slot belongs to this HWND and is valid until DestroyWindow returns.
    let cleanup = match unsafe { pointer.as_mut() } {
        Some(state) => {
            close_find_dialog(state);
            state.clear_accessibility()
        }
        None => Ok(()),
    };
    // SAFETY: All callers run on the UI thread that owns this live top-level HWND.
    let shutdown = unsafe { DestroyWindow(window) }
        .map_err(|error| ShellError::new(format!("window shutdown failed: {error}")));
    cleanup?;
    shutdown
}

fn scale(value: i32, dpi: u32) -> i32 {
    let scaled = i64::from(value) * i64::from(dpi) / 96;
    i32::try_from(scaled).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use super::{
        ActiveFind, MAX_COPY_ROWS, MAX_COPY_UTF16_UNITS, MAX_STARTUP_ERROR_UTF16_UNITS,
        NativeColumns, RevealedMatch, append_copy_units, build_copy_text, document_event_is_usable,
        document_smoke_evidence, document_title, failure_requires_reload, find_needle,
        is_supported_document_path, parse_one_based_row, query_match_to_reveal, query_status,
        startup_error_text, worker_status,
    };
    use crate::document_engine::{UiColumnKind, UiColumnLayout};
    use crate::worker::{
        WorkerEvent, WorkerPhase, WorkerProgress, WorkerQueryPhase, WorkerQueryState, WorkerResult,
    };
    use crate::{
        CachedRow, DisplayCell, DocumentSmokeColumnKind, DocumentSmokePhase, ImmutableRowCache,
    };

    fn ready_event() -> WorkerEvent {
        WorkerEvent {
            path: PathBuf::from("fixture.log"),
            result: WorkerResult::Ready { size: 12 },
            serial: 7,
            cache: Arc::new(ImmutableRowCache::new(vec![CachedRow::new(
                0,
                vec![DisplayCell::new("1"), DisplayCell::new("row value")],
            )])),
            cache_first_row: 0,
            cached_rows: 1,
            columns: Some(crate::document_engine::UiColumnLayout::preview()),
            query: None,
            progress: WorkerProgress {
                scanned_bytes: 0,
                source_bytes: 12,
                indexed_rows: 0,
                available_rows: 1,
                complete: false,
            },
            phase: WorkerPhase::ViewportReady,
            source_truncated_rows: 0,
            display_truncated_rows: 0,
            escaped_non_utf8_rows: 0,
        }
    }

    #[test]
    fn startup_error_text_is_nul_safe_and_bounded() {
        assert_eq!(
            startup_error_text("a\0b"),
            vec![u16::from(b'a'), u16::from(b'b'), 0]
        );
        let long = "x".repeat(MAX_STARTUP_ERROR_UTF16_UNITS * 2);
        let encoded = startup_error_text(&long);
        assert_eq!(encoded.len(), MAX_STARTUP_ERROR_UTF16_UNITS);
        assert_eq!(encoded.last(), Some(&0));
        assert!(encoded[..encoded.len() - 1].iter().all(|unit| *unit != 0));
    }

    #[test]
    fn document_smoke_evidence_uses_bounded_real_cache_cells() {
        let event = ready_event();
        assert!(document_event_is_usable(&event));
        let evidence = document_smoke_evidence(&event);
        assert_eq!(
            evidence.map(|value| (
                value.first_row,
                value.preview,
                value.column_kind,
                value.data_columns,
                value.sample_cells,
                value.phase,
                value.scan_complete
            )),
            Some((
                String::from("1"),
                String::from("row value"),
                DocumentSmokeColumnKind::Preview,
                1,
                vec![String::from("row value")],
                DocumentSmokePhase::ViewportReady,
                false
            ))
        );
    }

    #[test]
    fn document_smoke_evidence_attests_independent_csv_cells() {
        let mut event = ready_event();
        event.path = PathBuf::from("fixture.csv");
        event.cache = Arc::new(ImmutableRowCache::new(vec![CachedRow::new(
            0,
            vec![
                DisplayCell::new("1"),
                DisplayCell::new("alpha"),
                DisplayCell::new("quoted, value"),
                DisplayCell::new(""),
            ],
        )]));
        event.columns = UiColumnLayout::fields(3);
        assert!(document_event_is_usable(&event));
        let evidence = document_smoke_evidence(&event);
        assert_eq!(
            evidence.map(|value| (
                value.column_kind,
                value.data_columns,
                value.preview,
                value.sample_cells,
            )),
            Some((
                DocumentSmokeColumnKind::Fields,
                3,
                String::from("alpha"),
                vec![
                    String::from("alpha"),
                    String::from("quoted, value"),
                    String::new(),
                ],
            ))
        );
    }

    #[test]
    fn source_truncation_is_visible_in_native_status() {
        let mut event = ready_event();
        event.source_truncated_rows = 1;
        assert!(worker_status(&event, None).contains("1 source-truncated"));
    }

    #[test]
    fn find_input_is_bounded_unicode_encoded_as_utf8() -> Result<(), Box<dyn std::error::Error>> {
        let needle = find_needle(&[0x0072, 0x00E9, 0x0073, 0x0075, 0x006D, 0x00E9, 0])?;
        assert_eq!(needle, "r\u{e9}sum\u{e9}".as_bytes());
        assert!(find_needle(&[0]).is_err());
        assert!(find_needle(&[0xD800, 0]).is_err());
        Ok(())
    }

    #[test]
    fn query_status_and_match_reveal_are_explicit_and_stale_safe() {
        let query = WorkerQueryState {
            query_serial: 19,
            phase: WorkerQueryPhase::MatchReady,
            scanned_bytes: 50,
            source_bytes: 100,
            stored_hits: 4,
            active_hit_index: Some(2),
            active_row: Some(41),
            message: String::from("Match 3 (4 stored)."),
        };
        let status = query_status(&query, false);
        assert!(status.contains("match 3 at row 42"));
        assert!(status.contains("50%"));
        assert!(status.contains("raw UTF-8 bytes"));
        assert!(status.contains("ASCII-only case-insensitive; non-ASCII bytes remain exact"));
        let mut changed = query.clone();
        changed.phase = WorkerQueryPhase::Failed;
        changed.message = String::from("Search stopped because the source changed");
        assert!(query_status(&changed, false).contains("automatic reload is disabled"));

        let mut event = ready_event();
        event.query = Some(query);
        let active = ActiveFind {
            query_serial: 19,
            needle: b"row".to_vec(),
            case_sensitive: false,
        };
        assert_eq!(
            query_match_to_reveal(&event, Some(&active)),
            Some(RevealedMatch {
                query_serial: 19,
                hit_index: 2,
                absolute_row: 41,
            })
        );
        let stale = ActiveFind {
            query_serial: 20,
            needle: b"row".to_vec(),
            case_sensitive: false,
        };
        assert_eq!(query_match_to_reveal(&event, Some(&stale)), None);
    }

    #[test]
    fn one_based_row_parser_is_checked_across_u64_boundaries() {
        assert_eq!(parse_one_based_row(&[49]), Ok(0));
        assert_eq!(parse_one_based_row(&[49, 48, 48]), Ok(99));
        let maximum: Vec<u16> = u64::MAX.to_string().encode_utf16().collect();
        assert_eq!(parse_one_based_row(&maximum), Ok(u64::MAX - 1));
        let overflow: Vec<u16> = "18446744073709551616".encode_utf16().collect();
        assert!(parse_one_based_row(&overflow).is_err());
        assert!(parse_one_based_row(&[48]).is_err());
        assert!(parse_one_based_row(&[49, 32]).is_err());
        assert!(parse_one_based_row(&[]).is_err());
    }

    #[test]
    fn cache_copy_is_exact_tab_delimited_and_fails_closed_outside_cache()
    -> Result<(), Box<dyn std::error::Error>> {
        let cache = ImmutableRowCache::new(vec![
            CachedRow::new(
                7,
                vec![
                    DisplayCell::new("8"),
                    DisplayCell::new("alpha"),
                    DisplayCell::new(""),
                    DisplayCell::new("omega"),
                ],
            ),
            CachedRow::new(8, vec![DisplayCell::new("9"), DisplayCell::new("next")]),
        ]);
        let columns = NativeColumns {
            kind: Some(UiColumnKind::Fields),
            data_columns: 3,
        };
        let rows = crate::SlidingRowWindow::new(20, u32::MAX);
        let text = build_copy_text(&cache, rows, columns, &[7, 8])?;
        let expected: Vec<u16> = "8\talpha\t\tomega\r\n9\tnext\t\t\0"
            .encode_utf16()
            .collect();
        assert_eq!(text, expected);
        assert!(build_copy_text(&cache, rows, columns, &[6]).is_err());
        assert!(build_copy_text(&cache, rows, columns, &vec![7; MAX_COPY_ROWS + 1]).is_err());
        Ok(())
    }

    #[test]
    fn copy_size_limit_is_enforced_before_growth() -> Result<(), Box<dyn std::error::Error>> {
        let mut text = Vec::new();
        text.try_reserve_exact(MAX_COPY_UTF16_UNITS - 1)?;
        text.resize(MAX_COPY_UTF16_UNITS - 1, 65);
        assert!(append_copy_units(&mut text, &[66, 0]).is_err());
        assert_eq!(text.len(), MAX_COPY_UTF16_UNITS - 1);
        Ok(())
    }

    #[test]
    fn title_drop_extensions_and_reload_disclosure_are_exact()
    -> Result<(), Box<dyn std::error::Error>> {
        let encoded = document_title(Path::new(r"C:\data\résumé.CSV"))?;
        let payload = encoded
            .strip_suffix(&[0])
            .ok_or("title is not NUL-terminated")?;
        assert_eq!(String::from_utf16(payload)?, "résumé.CSV — LeanRows");
        assert!(is_supported_document_path(Path::new("rows.CSV")));
        assert!(is_supported_document_path(Path::new("events.ndjson")));
        assert!(!is_supported_document_path(Path::new("rows.parquet")));
        assert!(failure_requires_reload(
            "the open document changed (RevisionChanged)"
        ));
        assert!(failure_requires_reload(
            "the open document snapshot is no longer trustworthy"
        ));
        assert!(!failure_requires_reload("access was denied"));
        Ok(())
    }

    #[test]
    fn document_smoke_rejects_empty_and_inconsistent_events() {
        let mut event = ready_event();
        event.cache = Arc::new(ImmutableRowCache::default());
        event.cached_rows = 0;
        assert!(!document_event_is_usable(&event));

        let mut event = ready_event();
        event.progress.source_bytes = 13;
        assert!(!document_event_is_usable(&event));

        let mut event = ready_event();
        event.result = WorkerResult::Failed {
            message: String::from("fixture failure"),
        };
        event.phase = WorkerPhase::Failed;
        assert!(!document_event_is_usable(&event));
    }
}
