#![deny(unsafe_op_in_unsafe_fn)]

mod accessibility;
mod clipboard;
mod drop_files;
mod header;
mod layout;
mod process;
mod progress;
mod tabs;
mod theme;

use std::ffi::{OsString, c_void};
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    COLOR_WINDOW, CreateRoundRectRgn, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_SINGLELINE,
    DT_VCENTER, DeleteObject, DrawFocusRect, DrawTextW, FillRect, FillRgn, FrameRgn, GetDC,
    GetPixel, GetSysColorBrush, HDC, HGDIOBJ, InvalidateRect, RDW_ALLCHILDREN, RDW_ERASE,
    RDW_INVALIDATE, RDW_UPDATENOW, RedrawWindow, ReleaseDC, SelectObject, SetBkColor, SetBkMode,
    SetTextColor, TRANSPARENT, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::{FindResourceW, GetModuleHandleW};
use windows::Win32::System::SystemServices::{
    SS_CENTER, SS_CENTERIMAGE, SS_ICON, SS_OWNERDRAW, SS_TYPEMASK,
};
use windows::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, FNERR_BUFFERTOOSMALL, GetOpenFileNameW, OFN_ALLOWMULTISELECT,
    OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::Controls::{
    BST_CHECKED, CDDS_PREPAINT, CDIS_DISABLED, CDIS_FOCUS, CDIS_HOT, CDRF_SKIPDEFAULT,
    DRAWITEMSTRUCT, EM_SETCUEBANNER, EM_SETSEL, HDM_GETITEMCOUNT, ICC_BAR_CLASSES,
    ICC_LISTVIEW_CLASSES, ICC_PROGRESS_CLASS, ICC_TAB_CLASSES, INITCOMMONCONTROLSEX,
    InitCommonControlsEx, LIST_VIEW_ITEM_STATE_FLAGS, LVCF_SUBITEM, LVCF_TEXT, LVCF_WIDTH,
    LVCFMT_LEFT, LVCOLUMNW, LVIF_TEXT, LVIS_FOCUSED, LVIS_SELECTED, LVITEMW, LVM_DELETECOLUMN,
    LVM_ENSUREVISIBLE, LVM_GETBKCOLOR, LVM_GETCOLUMNWIDTH, LVM_GETCOUNTPERPAGE, LVM_GETHEADER,
    LVM_GETNEXTITEM, LVM_GETTOPINDEX, LVM_INSERTCOLUMNW, LVM_REDRAWITEMS, LVM_SETBKCOLOR,
    LVM_SETCOLUMNWIDTH, LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SETITEMCOUNT, LVM_SETITEMSTATE,
    LVM_SETTEXTBKCOLOR, LVM_SETTEXTCOLOR, LVN_GETDISPINFOW, LVN_ODCACHEHINT, LVNI_SELECTED,
    LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT, LVS_OWNERDATA, LVS_REPORT, LVS_SHOWSELALWAYS,
    LVSICF_NOINVALIDATEALL, LVSICF_NOSCROLL, NM_CUSTOMDRAW, NMCUSTOMDRAW, NMHDR, NMLVCACHEHINT,
    NMLVDISPINFOW, ODS_DISABLED, ODS_FOCUS, ODS_NOFOCUSRECT, ODS_SELECTED, ODT_BUTTON, ODT_STATIC,
    PBM_GETPOS, PBM_SETRANGE32, PBS_MARQUEE, PROGRESS_CLASSW, SetWindowTheme, TCN_SELCHANGE,
    TCS_FIXEDWIDTH, WC_LISTVIEWW, WC_TABCONTROLW,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, IsWindowEnabled, SetFocus, VK_CONTROL, VK_ESCAPE, VK_RETURN, VK_SHIFT,
    VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, BM_CLICK, BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_OWNERDRAW, BS_TYPEMASK,
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyAcceleratorTable, DestroyMenu, DestroyWindow, DialogBoxParamW,
    DispatchMessageW, ES_AUTOHSCROLL, EndDialog, GCLP_HICON, GCLP_HICONSM, GWL_STYLE,
    GWLP_USERDATA, GetClassLongPtrW, GetClassNameW, GetClientRect, GetDlgItemTextW, GetMessageW,
    GetSystemMetrics, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    HACCEL, HICON, HMENU, HWND_TOP, IDC_ARROW, IDCANCEL, IDOK, IMAGE_ICON, IsIconic, IsWindow,
    IsWindowVisible, KillTimer, LR_SHARED, LoadAcceleratorsW, LoadCursorW, LoadImageW,
    MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_TASKMODAL, MF_CHECKED, MF_GRAYED,
    MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MINMAXINFO, MSG, MessageBoxW, MoveWindow, PostMessageW,
    PostQuitMessage, RT_DIALOG, RegisterClassExW, SIZE_MINIMIZED, SM_CXICON, SM_CXSMICON,
    SM_CYICON, SM_CYSMICON, STM_SETICON, SW_HIDE, SW_RESTORE, SW_SHOW, SW_SHOWDEFAULT,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SendMessageW, SetDlgItemTextW,
    SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow,
    TPM_RETURNCMD, TPM_RIGHTALIGN, TrackPopupMenuEx, TranslateAcceleratorW, TranslateMessage,
    WINDOW_EX_STYLE, WINDOW_LONG_PTR_INDEX, WINDOW_STYLE, WM_APP, WM_CLOSE, WM_COMMAND,
    WM_COPYDATA, WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY,
    WM_DPICHANGED, WM_DRAWITEM, WM_DROPFILES, WM_ERASEBKGND, WM_GETFONT, WM_GETMINMAXINFO,
    WM_INITDIALOG, WM_KEYDOWN, WM_NCCREATE, WM_NCDESTROY, WM_NOTIFY, WM_PRINTCLIENT, WM_SETFONT,
    WM_SETTINGCHANGE, WM_SIZE, WM_SYSCOLORCHANGE, WM_THEMECHANGED, WM_TIMER, WNDCLASSEXW,
    WS_BORDER, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_ACCEPTFILES, WS_EX_APPWINDOW,
    WS_OVERLAPPEDWINDOW, WS_TABSTOP,
};
use windows::core::{PCWSTR, PWSTR, w};

use self::accessibility::{
    AccessibilityBridge, ComApartment, SmokeEvidence, smoke_cache, verify_smoke,
};
use self::layout::{CommandLayoutMode, UiLayout, UiRect, tab_item_width};
use self::tabs::TabHover;
use self::theme::{EffectiveTheme, ThemeMode, ThemeResources};
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
const WM_APP_PRESENT_SHELL: u32 = WM_APP + 2;
const WM_APP_OPEN_PENDING: u32 = WM_APP + 3;
const WM_APP_CLOSE_TAB: u32 = WM_APP + 4;
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
const ID_VIEW_THEME: u16 = 120;
const ID_SEARCH_MATCH_CASE: u16 = 121;
const ID_APP_MORE: u16 = 122;
const ID_TAB_CLOSE: u16 = 130;
const ID_TAB_NEXT: u16 = 131;
const ID_TAB_PREVIOUS: u16 = 132;
const ID_TAB_SELECT_FIRST: u16 = 141;
const ID_TAB_SELECT_LAST: u16 = 149;
const ID_HELP_ABOUT: u16 = 200;
const ID_SEARCH_FIELD: u16 = 300;
const ID_TAB_STRIP: u16 = 301;
const IDD_GOTO_ROW: usize = 201;
const IDC_GOTO_ROW_EDIT: i32 = 1001;
const MAX_COPY_ROWS: usize = 4_096;
const MAX_COPY_BYTES: usize = 1_024 * 1_024;
const MAX_COPY_UTF16_UNITS: usize = MAX_COPY_BYTES / size_of::<u16>();
const MAX_FIND_UTF16_UNITS: usize = 1_024;
const EMPTY_TITLE_CAPTION: &str = "Open large files without loading them all.";
const WELCOME_EYEBROW: &str = "LEANROWS";
const WELCOME_BODY: &str = "Open a local CSV, TSV, JSONL, NDJSON, log, or text file. LeanRows keeps memory bounded and never modifies the source.";
const IDLE_FILE_NAME: &str = "LeanRows";
const IDLE_FILE_META: &str = "Large-file row viewer";
const IDLE_STATUS: &str = "Ready";
const WINDOW_CLASS_NAME: PCWSTR = w!("LeanRows.NativeWindow");
const FIRST_TAB_ID: usize = 1;
/// Each open file keeps a worker thread, a file handle, and one bounded row
/// window, so the number of tabs is capped to keep memory bounded.
const MAX_OPEN_TABS: usize = 32;
const MAX_CHROME_TEXT_UNITS: usize = 4_096;
const SHELL_SMOKE_TEXT_UNITS: usize = 64;
const STATUS_PAINT_TEXT_UNITS: usize = 1_024;
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
}

impl Drop for MenuGuard {
    fn drop(&mut self) {
        if let Some(menu) = self.0.take() {
            // SAFETY: The guard owns menus until ownership transfers to a parent/window.
            let _ = unsafe { DestroyMenu(menu) };
        }
    }
}

#[derive(Default)]
struct ChromeHandles {
    topbar: HWND,
    wordmark: HWND,
    file_name: HWND,
    file_meta: HWND,
    search_edit: HWND,
    match_case: HWND,
    find_previous: HWND,
    find_next: HWND,
    reload: HWND,
    goto: HWND,
    theme: HWND,
    open: HWND,
    more: HWND,
    tab_strip: HWND,
    progress: HWND,
    empty_eyebrow: HWND,
    empty_title: HWND,
    empty_body: HWND,
    empty_open: HWND,
    status_backdrop: HWND,
}

#[allow(
    clippy::struct_excessive_bools,
    reason = "grid, progress, smoke, and closing are independent window states"
)]
struct WindowState {
    window: HWND,
    list: HWND,
    status: HWND,
    chrome: ChromeHandles,
    theme: ThemeResources,
    grid_visible: bool,
    progress_visible: bool,
    rows: SlidingRowWindow,
    cache: Arc<ImmutableRowCache>,
    accessibility: AccessibilityBridge,
    worker: Option<Worker>,
    current_path: Option<PathBuf>,
    active_serial: Option<u64>,
    columns: NativeColumns,
    pending_reveal: Option<u64>,
    active_find: Option<ActiveFind>,
    revealed_match: Option<RevealedMatch>,
    /// The active tab. Its document lives in the fields above, so the grid,
    /// find, and copy code only ever see one document.
    tab_id: usize,
    next_tab_id: usize,
    /// Every open tab, left to right, including the active one.
    tab_order: Vec<usize>,
    /// Documents in the tabs that are not showing.
    background_tabs: Vec<BackgroundTab>,
    tab_labels: Vec<String>,
    tab_hover: TabHover,
    /// Files handed over by another launch, opened once the sender returns.
    pending_opens: Vec<PathBuf>,
    /// Set once the window starts closing, so forwarded files go elsewhere.
    closing: bool,
    document_smoke: bool,
    document_smoke_observed: Arc<AtomicU64>,
    document_smoke_evidence: Arc<Mutex<Option<DocumentSmokeEvidence>>>,
    reclaimed: Arc<AtomicBool>,
}

struct GoToDialogState {
    initial_row: u64,
    selected_row: Option<u64>,
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

    /// Records a worker layout for a tab that is not showing. The rules match
    /// `synchronize_data_columns`, which applies them to the live list.
    fn grow(&mut self, layout: UiColumnLayout) -> Result<(), ShellError> {
        if !layout.is_valid() {
            return Err(ShellError::new("worker supplied invalid column metadata"));
        }
        match self.kind {
            Some(kind) if kind != layout.kind() => {
                return Err(ShellError::new(
                    "worker changed the column kind within one document",
                ));
            }
            None => self.kind = Some(layout.kind()),
            Some(_) => {}
        }
        self.data_columns = self.data_columns.max(layout.data_columns());
        Ok(())
    }
}

/// A document open in a tab that is not showing. Its worker keeps scanning.
/// The cached rows are the allocation the worker also holds, so keeping them
/// here costs no extra memory and switching back shows rows at once.
struct BackgroundTab {
    id: usize,
    worker: Option<Worker>,
    path: Option<PathBuf>,
    serial: Option<u64>,
    columns: NativeColumns,
    column_widths: Vec<i32>,
    rows: SlidingRowWindow,
    cache: Arc<ImmutableRowCache>,
    top_row: Option<u64>,
    selected_row: Option<u64>,
    pending_reveal: Option<u64>,
    active_find: Option<ActiveFind>,
    revealed_match: Option<RevealedMatch>,
    chrome: TabChrome,
}

impl BackgroundTab {
    /// Keeps the tab's chrome and rows current while another tab is showing.
    fn apply_event(&mut self, event: &WorkerEvent) {
        if self.path.as_ref() != Some(&event.path) || self.serial != Some(event.serial) {
            return;
        }
        let chrome = event_chrome(event);
        self.chrome.file_meta = file_meta_text(chrome.source_bytes);
        self.chrome.grid_visible = chrome.grid_visible;
        self.chrome.progress_visible = chrome.progress_visible;
        self.chrome.progress_percent = chrome.progress_percent;
        if let Some([eyebrow, title, body]) = chrome.empty_state {
            eyebrow.clone_into(&mut self.chrome.empty_eyebrow);
            title.clone_into(&mut self.chrome.empty_title);
            body.clone_into(&mut self.chrome.empty_body);
        }
        if let Some(layout) = event.columns
            && let Err(error) = self.columns.grow(layout)
        {
            self.chrome.status = format!("Column update failed: {error}");
            return;
        }
        self.chrome.status = worker_status(event, self.active_find.as_ref());
        let mut rows = SlidingRowWindow::new(event.progress.available_rows, u32::MAX);
        rows.seek(self.rows.first_row());
        self.rows = rows;
        self.cache = Arc::clone(&event.cache);
        if let Some(reveal) = query_match_to_reveal(event, self.active_find.as_ref())
            && self.revealed_match != Some(reveal)
        {
            self.revealed_match = Some(reveal);
            self.pending_reveal = Some(reveal.absolute_row);
        }
    }
}

/// The text and indicators one tab shows in the shared chrome.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct TabChrome {
    file_name: String,
    file_meta: String,
    empty_eyebrow: String,
    empty_title: String,
    empty_body: String,
    status: String,
    grid_visible: bool,
    progress_visible: bool,
    progress_percent: u64,
}

/// What one worker event shows in the chrome, for the active tab or for a
/// tab that is not showing.
#[derive(Debug, Eq, PartialEq)]
struct EventChrome<'a> {
    source_bytes: Option<u64>,
    grid_visible: bool,
    progress_visible: bool,
    progress_percent: u64,
    /// Eyebrow, title, and body for the empty state when the grid is hidden.
    empty_state: Option<[&'a str; 3]>,
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
    // Automation stays isolated. An ordinary launch hands its files to the
    // LeanRows window that is already running, when there is one.
    let _instance = if shell_smoke {
        None
    } else {
        match process::claim(&options.initial_paths) {
            process::Startup::Forwarded => return Ok(ShellOutcome::default()),
            process::Startup::Primary(guard) => guard,
        }
    };
    let initial_paths = options.initial_paths;

    initialize_native_controls()?;
    let _com_apartment = ComApartment::initialize()?;
    let accessibility = AccessibilityBridge::new()?;
    // SAFETY: This reads the system DPI before any top-level HWND is created.
    let initial_dpi = unsafe { GetDpiForSystem() }.max(96);
    let theme = ThemeResources::new(ThemeMode::load(), initial_dpi)?;

    // SAFETY: Passing `None` requests the current executable module.
    let module = unsafe { GetModuleHandleW(None) }
        .map_err(|error| ShellError::new(format!("module lookup failed: {error}")))?;
    let instance = HINSTANCE(module.0);
    register_window_class(instance)?;
    let accelerators = load_accelerators(instance)?;
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
        chrome: ChromeHandles::default(),
        theme,
        grid_visible: shell_smoke,
        progress_visible: false,
        rows: SlidingRowWindow::new(u64::from(shell_smoke), u32::from(shell_smoke)),
        cache,
        accessibility,
        worker: None,
        current_path: None,
        active_serial: None,
        columns: NativeColumns::preview(),
        pending_reveal: None,
        active_find: None,
        revealed_match: None,
        tab_id: FIRST_TAB_ID,
        next_tab_id: FIRST_TAB_ID + 1,
        tab_order: vec![FIRST_TAB_ID],
        background_tabs: Vec::new(),
        tab_labels: Vec::new(),
        tab_hover: TabHover::default(),
        pending_opens: Vec::new(),
        closing: false,
        document_smoke: options.document_smoke_test,
        document_smoke_observed: Arc::clone(&document_smoke_observed),
        document_smoke_evidence: Arc::clone(&document_smoke_evidence),
        reclaimed: Arc::clone(&reclaimed),
    });
    let (window, state_pointer) = create_shell_window(instance, state, reclaimed.as_ref())?;
    initialize_window_title(window)?;

    let smoke_evidence = if options.smoke_test {
        run_smoke(window, state_pointer, &smoke_verified)?
    } else if options.document_smoke_test {
        start_document_smoke(window, state_pointer, &smoke_verified, initial_paths)?
    } else {
        open_documents(window, initial_paths);
        post_initial_presentation(window)?;
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

fn start_document_smoke(
    window: HWND,
    state_pointer: *mut WindowState,
    smoke_verified: &AtomicBool,
    initial_paths: Vec<PathBuf>,
) -> Result<SmokeEvidence, ShellError> {
    let evidence = match verify_shell_smoke(window, state_pointer, smoke_verified) {
        Ok(evidence) => evidence,
        Err(error) => {
            let _ = destroy_shell_window(window);
            return Err(error);
        }
    };
    let path = initial_paths.into_iter().next().ok_or_else(|| {
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
    Ok(evidence)
}

fn initialize_window_title(window: HWND) -> Result<(), ShellError> {
    // Explicitly restore the product caption after child/theme initialization;
    // document opens replace it with the file-specific title below.
    unsafe { SetWindowTextW(window, w!("LeanRows")) }
        .map_err(|error| ShellError::new(format!("window title initialization failed: {error}")))
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
    if options.document_smoke_test && options.initial_paths.len() != 1 {
        return Err(ShellError::new(
            "document smoke requires exactly one input file",
        ));
    }
    Ok(options.smoke_test || options.document_smoke_test)
}

fn initialize_native_controls() -> Result<(), ShellError> {
    let controls_size = u32::try_from(size_of::<INITCOMMONCONTROLSEX>())
        .map_err(|_| ShellError::new("common-control structure size overflow"))?;
    let controls = INITCOMMONCONTROLSEX {
        dwSize: controls_size,
        dwICC: ICC_LISTVIEW_CLASSES | ICC_BAR_CLASSES | ICC_PROGRESS_CLASS | ICC_TAB_CLASSES,
    };
    // SAFETY: `controls` is initialized to the documented structure size and flags.
    if !unsafe { InitCommonControlsEx(&raw const controls) }.as_bool() {
        return Err(ShellError::new("common-control initialization failed"));
    }
    Ok(())
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
            false,
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
            WINDOW_CLASS_NAME,
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
    post_initial_presentation(window)?;
    dispatch_initial_presentation(window)?;
    // SAFETY: This handle was created on the current UI thread. A failed
    // first presentation may already have destroyed it and reclaimed the
    // attached state, so validate both the HWND and pointer slot before any
    // raw-pointer dereference.
    if !unsafe { IsWindow(Some(window)) }.as_bool() || state_pointer_for(window) != state_pointer {
        return Err(ShellError::new(
            "initial presentation destroyed or detached the smoke window",
        ));
    }
    verify_initial_surface_colors(state_pointer)?;
    let verification = verify_shell_smoke(window, state_pointer, controls_verified);
    let shutdown = destroy_shell_window(window)
        .map_err(|error| ShellError::new(format!("smoke shutdown failed: {error}")));
    let evidence = verification?;
    shutdown?;
    Ok(evidence)
}

fn dispatch_initial_presentation(window: HWND) -> Result<(), ShellError> {
    let mut message = MSG::default();
    // SAFETY: The writable message is filtered to the pointer-free one-shot
    // presentation message posted for this exact live window.
    let result = unsafe {
        GetMessageW(
            &raw mut message,
            Some(window),
            WM_APP_PRESENT_SHELL,
            WM_APP_PRESENT_SHELL,
        )
    };
    if result.0 == -1 {
        return Err(ShellError::new(format!(
            "initial presentation retrieval failed: {}",
            windows::core::Error::from_thread()
        )));
    }
    if !result.as_bool() {
        return Err(ShellError::new(
            "initial presentation was interrupted by an unexpected quit",
        ));
    }
    // SAFETY: GetMessageW populated this exact filtered message successfully.
    unsafe { DispatchMessageW(&raw const message) };
    Ok(())
}

fn verify_initial_surface_colors(state_pointer: *mut WindowState) -> Result<(), ShellError> {
    // SAFETY: The initial presentation message leaves the attached state live.
    let state = unsafe { state_pointer.as_ref() }
        .ok_or_else(|| ShellError::new("initial surface smoke state was detached"))?;
    for (handle, expected, name) in [
        (
            state.chrome.topbar,
            state.theme.palette.surface.colorref().0,
            "top bar",
        ),
        (
            state.chrome.status_backdrop,
            state.theme.palette.surface_muted.colorref().0,
            "status backdrop",
        ),
        (
            state.status,
            state.theme.palette.surface_muted.colorref().0,
            "status text",
        ),
    ] {
        verify_child_background_pixel(handle, expected, name)?;
    }
    Ok(())
}

fn verify_child_background_pixel(
    handle: HWND,
    expected: u32,
    name: &str,
) -> Result<(), ShellError> {
    let mut client = RECT::default();
    // SAFETY: The child is live after initial presentation and RECT is writable.
    unsafe { GetClientRect(handle, &raw mut client) }
        .map_err(|error| ShellError::new(format!("{name} geometry query failed: {error}")))?;
    let width = client.right.saturating_sub(client.left);
    let height = client.bottom.saturating_sub(client.top);
    if width < 8 || height < 8 {
        return Err(ShellError::new(format!(
            "{name} is too small for bounded background verification"
        )));
    }
    // SAFETY: The child owns this DC until the paired ReleaseDC below.
    let device = unsafe { GetDC(Some(handle)) };
    if device.0.is_null() {
        return Err(ShellError::new(format!(
            "{name} background verification could not acquire a DC"
        )));
    }
    // Four pixels in from the upper-right corner avoids text, glyphs, and borders.
    let actual = unsafe { GetPixel(device, width.saturating_sub(4), 4) }.0;
    // SAFETY: This releases the exact HWND/DC pair acquired above.
    let released = unsafe { ReleaseDC(Some(handle), device) };
    if released == 0 {
        return Err(ShellError::new(format!(
            "{name} background verification could not release its DC"
        )));
    }
    if actual != expected {
        return Err(ShellError::new(format!(
            "{name} did not paint the active theme on first presentation"
        )));
    }
    Ok(())
}

fn verify_shell_smoke(
    window: HWND,
    state_pointer: *mut WindowState,
    controls_verified: &AtomicBool,
) -> Result<SmokeEvidence, ShellError> {
    show_shell_window(window)?;
    // SAFETY: Window creation normally attaches this pointer through WM_NCCREATE.
    if let Some(state) = unsafe { state_pointer.as_mut() } {
        // SAFETY: Both handles were created on this UI thread and are queried read-only.
        let verified = unsafe { IsWindow(Some(state.list)) }.as_bool()
            && unsafe { IsWindow(Some(state.status)) }.as_bool();
        if verified {
            verify_modern_shell_chrome(state)?;
            verify_theme_button_cycle(window)?;
            verify_progress_indicator(state)?;
            verify_tab_strip(window)?;
            verify_native_presentation(window, state)?;
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

fn verify_modern_shell_chrome(state: &WindowState) -> Result<(), ShellError> {
    for (handle, name) in [
        (state.chrome.topbar, "top bar"),
        (state.chrome.search_edit, "inline search"),
        (state.chrome.open, "primary Open button"),
        (state.chrome.more, "overflow button"),
        (state.chrome.status_backdrop, "status backdrop"),
        (state.status, "status text"),
    ] {
        verify_control_visibility(handle, name, true)?;
    }
    for (handle, name) in [
        (state.chrome.empty_title, "empty-state title"),
        (state.chrome.empty_open, "empty-state Open button"),
        (state.chrome.progress, "progress indicator"),
        (state.chrome.tab_strip, "tab strip"),
    ] {
        verify_control_visibility(handle, name, false)?;
    }
    for handle in [
        state.chrome.find_previous,
        state.chrome.find_next,
        state.chrome.reload,
        state.chrome.goto,
        state.chrome.theme,
        state.chrome.open,
        state.chrome.more,
        state.chrome.empty_open,
    ] {
        // SAFETY: The handle is a live BUTTON child queried read-only during smoke.
        let style = unsafe { GetWindowLongPtrW(handle, GWL_STYLE) };
        let type_mask = isize::try_from(BS_TYPEMASK).unwrap_or_default();
        let owner_draw = isize::try_from(BS_OWNERDRAW).unwrap_or_default();
        if style & type_mask != owner_draw {
            return Err(ShellError::new(
                "modern shell command buttons are not owner-drawn",
            ));
        }
    }
    let static_type_mask = isize::try_from(SS_TYPEMASK.0).unwrap_or_default();
    let static_owner_draw = isize::try_from(SS_OWNERDRAW.0).unwrap_or_default();
    for handle in [
        state.chrome.topbar,
        state.chrome.status_backdrop,
        state.status,
    ] {
        // SAFETY: These are live STATIC children queried read-only.
        let style = unsafe { GetWindowLongPtrW(handle, GWL_STYLE) };
        if style & static_type_mask != static_owner_draw {
            return Err(ShellError::new(
                "modern shell background surfaces are not deterministically owner-drawn",
            ));
        }
    }

    let mut title = [0_u16; SHELL_SMOKE_TEXT_UNITS];
    // SAFETY: The hidden title control is live and the fixed buffer is writable.
    let copied = unsafe { GetWindowTextW(state.chrome.empty_title, &mut title) };
    let copied = usize::try_from(copied).unwrap_or_default().min(title.len());
    let expected = EMPTY_TITLE_CAPTION.encode_utf16();
    if copied != expected.clone().count() || !title[..copied].iter().copied().eq(expected) {
        return Err(ShellError::new(
            "modern shell empty-state title caption changed unexpectedly",
        ));
    }
    Ok(())
}

fn verify_theme_button_cycle(window: HWND) -> Result<(), ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: Smoke runs synchronously on the window's owning UI thread.
    let original = unsafe { pointer.as_ref() }
        .ok_or_else(|| ShellError::new("theme smoke state is unavailable"))?
        .theme
        .preference;
    let verification = (|| {
        refresh_theme(window, ThemeMode::System)?;
        verify_theme_runtime_state(window, ThemeMode::System)?;
        for expected in [ThemeMode::Light, ThemeMode::Dark, ThemeMode::System] {
            // SAFETY: BM_CLICK performs the real accessible BUTTON activation and
            // synchronously routes ID_VIEW_THEME through the parent window.
            unsafe {
                SendMessageW(
                    pointer
                        .as_ref()
                        .ok_or_else(|| ShellError::new("theme button state was detached"))?
                        .chrome
                        .theme,
                    BM_CLICK,
                    None,
                    None,
                );
            }
            verify_theme_runtime_state(window, expected)?;
        }
        Ok(())
    })();
    let restoration = refresh_theme(window, original);
    if let Err(error) = restoration {
        return Err(ShellError::new(format!(
            "theme smoke could not restore the original preference: {error}"
        )));
    }
    verification
}

fn verify_theme_runtime_state(window: HWND, expected: ThemeMode) -> Result<(), ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: The state remains attached throughout synchronous smoke execution.
    let state = unsafe { pointer.as_ref() }
        .ok_or_else(|| ShellError::new("theme runtime state is unavailable"))?;
    let expected_effective = ThemeResources::new(expected, state.theme.metrics.dpi)?.effective;
    if state.theme.preference != expected || state.theme.effective != expected_effective {
        return Err(ShellError::new(format!(
            "theme button did not apply the expected {expected:?} runtime state"
        )));
    }
    if ThemeMode::load() != expected {
        return Err(ShellError::new(format!(
            "theme button did not persist the expected {expected:?} preference"
        )));
    }
    verify_control_text(state.chrome.theme, expected.label(), "theme button")?;
    // SAFETY: LVM_GETBKCOLOR is a pointer-free read of the live list-view palette.
    let list_background = unsafe { SendMessageW(state.list, LVM_GETBKCOLOR, None, None) }.0;
    let expected_background =
        isize::try_from(state.theme.palette.surface.colorref().0).unwrap_or_default();
    if list_background != expected_background {
        return Err(ShellError::new(
            "theme button did not apply its surface color to the row grid",
        ));
    }
    // SAFETY: The top-level window and children repaint synchronously here.
    let _ = unsafe { UpdateWindow(window) };
    Ok(())
}

fn verify_control_text(handle: HWND, expected: &str, name: &str) -> Result<(), ShellError> {
    let mut text = [0_u16; SHELL_SMOKE_TEXT_UNITS];
    // SAFETY: handle is a live child and the fixed local buffer is writable.
    let copied = unsafe { GetWindowTextW(handle, &mut text) };
    let copied = usize::try_from(copied).unwrap_or_default().min(text.len());
    let expected = expected.encode_utf16();
    if copied != expected.clone().count() || !text[..copied].iter().copied().eq(expected) {
        return Err(ShellError::new(format!(
            "modern shell {name} caption changed unexpectedly"
        )));
    }
    Ok(())
}

fn verify_progress_indicator(state: &WindowState) -> Result<(), ShellError> {
    let progress = state.chrome.progress;
    let mut class_name = [0_u16; 32];
    // SAFETY: progress is a live child and the bounded class buffer is writable.
    let class_units = unsafe { GetClassNameW(progress, &mut class_name) };
    let class_units = usize::try_from(class_units)
        .unwrap_or_default()
        .min(class_name.len());
    if !class_name[..class_units]
        .iter()
        .copied()
        .eq("msctls_progress32".encode_utf16())
    {
        return Err(ShellError::new(
            "flat progress rule no longer preserves the native progress class",
        ));
    }
    // SAFETY: This is a read-only style query for the live progress child.
    let style = unsafe { GetWindowLongPtrW(progress, GWL_STYLE) };
    if style & isize::try_from(PBS_MARQUEE).unwrap_or_default() != 0 {
        return Err(ShellError::new(
            "flat progress rule unexpectedly retained marquee styling",
        ));
    }
    let mut client = RECT::default();
    // SAFETY: The child is live and the local RECT is writable.
    unsafe { GetClientRect(progress, &raw mut client) }
        .map_err(|error| ShellError::new(format!("progress geometry query failed: {error}")))?;
    let width = client.right.saturating_sub(client.left);
    let height = client.bottom.saturating_sub(client.top);
    if width < 8 || height != state.theme.metrics.progress_height {
        return Err(ShellError::new(
            "flat progress rule does not match the LeanMark-aligned geometry",
        ));
    }

    // SAFETY: The child belongs to this UI thread and is temporarily revealed
    // solely for bounded pixel/value verification.
    let _ = unsafe { ShowWindow(progress, SW_SHOW) };
    progress::update_position(progress, 50);
    let verification = (|| {
        // SAFETY: PBM_GETPOS is a pointer-free semantic value query.
        let position = unsafe { SendMessageW(progress, PBM_GETPOS, None, None) }.0;
        if position != 50 {
            return Err(ShellError::new(
                "native progress accessibility value did not reach 50 percent",
            ));
        }
        verify_progress_pixels(state, width, height)
    })();
    progress::update_position(progress, 0);
    // SAFETY: Restore the loaded-smoke visibility contract unconditionally.
    let _ = unsafe { ShowWindow(progress, SW_HIDE) };
    verification?;
    verify_control_visibility(progress, "progress indicator", false)
}

fn verify_progress_pixels(state: &WindowState, width: i32, height: i32) -> Result<(), ShellError> {
    let progress = state.chrome.progress;
    // SAFETY: The live child owns this DC until the paired ReleaseDC below.
    let device = unsafe { GetDC(Some(progress)) };
    if device.0.is_null() {
        return Err(ShellError::new(
            "progress pixel smoke could not acquire a DC",
        ));
    }
    // SAFETY: WM_PRINTCLIENT synchronously paints into the supplied live HDC.
    unsafe {
        SendMessageW(
            progress,
            WM_PRINTCLIENT,
            Some(WPARAM(device.0.addr())),
            Some(LPARAM(0)),
        );
    }
    let prefix = unsafe { GetPixel(device, width / 4, 0) };
    let suffix = unsafe { GetPixel(device, width.saturating_mul(3) / 4, 0) };
    let lower_suffix = unsafe {
        GetPixel(
            device,
            width.saturating_mul(3) / 4,
            height.saturating_sub(1),
        )
    };
    // SAFETY: This releases the exact HWND/DC pair acquired above.
    let released = unsafe { ReleaseDC(Some(progress), device) };
    if released == 0 {
        return Err(ShellError::new(
            "progress pixel smoke could not release its DC",
        ));
    }
    if prefix != state.theme.palette.accent.colorref()
        || suffix != state.theme.palette.surface.colorref()
        || lower_suffix != state.theme.palette.border.colorref()
    {
        return Err(ShellError::new(
            "flat progress pixels do not match the LeanMark accent/surface/border contract",
        ));
    }
    Ok(())
}

/// Lays the shell out as it is with two open files, checks that the strip
/// holds both tabs and paints the selected one, then restores the one-file
/// layout.
fn verify_tab_strip(window: HWND) -> Result<(), ShellError> {
    const SMOKE_TAB_ID: usize = usize::MAX;

    let pointer = state_pointer_for(window);
    // SAFETY: Smoke runs synchronously on the window's owning UI thread.
    let strip = unsafe { pointer.as_mut() }
        .map(|state| {
            state.tab_order.push(SMOKE_TAB_ID);
            state.chrome.tab_strip
        })
        .ok_or_else(|| ShellError::new("tab strip smoke state is unavailable"))?;
    layout_children(window);
    tabs::set_items(
        strip,
        &[String::from("first.csv"), String::from("second.csv")],
        1,
    );
    // SAFETY: As above; layout and item updates leave the state attached.
    let verification = unsafe { pointer.as_ref() }
        .ok_or_else(|| ShellError::new("tab strip smoke state was detached"))
        .and_then(verify_tab_strip_pixels);
    // SAFETY: As above.
    if let Some(state) = unsafe { pointer.as_mut() } {
        state.tab_order.retain(|id| *id != SMOKE_TAB_ID);
    }
    tabs::set_items(strip, &[], 0);
    layout_children(window);
    verification?;
    verify_control_visibility(strip, "tab strip", false)
}

fn verify_tab_strip_pixels(state: &WindowState) -> Result<(), ShellError> {
    let strip = state.chrome.tab_strip;
    verify_control_visibility(strip, "two-tab strip", true)?;
    if tabs::item_count(strip) != 2 || tabs::selected_index(strip) != Some(1) {
        return Err(ShellError::new(
            "tab strip did not keep two tabs with the second selected",
        ));
    }
    let (Some(first), Some(second)) = (tabs::item_rect(strip, 0), tabs::item_rect(strip, 1)) else {
        return Err(ShellError::new("tab strip geometry query failed"));
    };
    let mut client = RECT::default();
    // SAFETY: The strip is live and the RECT is writable.
    unsafe { GetClientRect(strip, &raw mut client) }
        .map_err(|error| ShellError::new(format!("tab strip geometry query failed: {error}")))?;
    let bottom = client.bottom.saturating_sub(1);
    // SAFETY: The live strip owns this DC until the paired ReleaseDC below.
    let device = unsafe { GetDC(Some(strip)) };
    if device.0.is_null() {
        return Err(ShellError::new(
            "tab strip pixel smoke could not acquire a DC",
        ));
    }
    // SAFETY: WM_PRINTCLIENT synchronously paints into the supplied live HDC.
    unsafe {
        SendMessageW(
            strip,
            WM_PRINTCLIENT,
            Some(WPARAM(device.0.addr())),
            Some(LPARAM(0)),
        );
    }
    let background = unsafe { GetPixel(device, client.right.saturating_sub(4), 4) };
    let selected = unsafe { GetPixel(device, second.left.midpoint(second.right), bottom) };
    let unselected = unsafe { GetPixel(device, first.left.midpoint(first.right), bottom) };
    // SAFETY: This releases the exact HWND/DC pair acquired above.
    let released = unsafe { ReleaseDC(Some(strip), device) };
    if released == 0 {
        return Err(ShellError::new(
            "tab strip pixel smoke could not release its DC",
        ));
    }
    if background != state.theme.palette.surface.colorref()
        || selected != state.theme.palette.accent.colorref()
        || unselected != state.theme.palette.border.colorref()
    {
        return Err(ShellError::new(
            "tab strip pixels do not match the surface/accent/border contract",
        ));
    }
    Ok(())
}

fn verify_control_visibility(
    handle: HWND,
    name: &str,
    expected_visible: bool,
) -> Result<(), ShellError> {
    // SAFETY: These are read-only queries against UI-thread-owned child handles.
    let exists = unsafe { IsWindow(Some(handle)) }.as_bool();
    let visible = exists && unsafe { IsWindowVisible(handle) }.as_bool();
    if !exists || visible != expected_visible {
        return Err(ShellError::new(format!(
            "modern shell {name} handle or visibility verification failed"
        )));
    }
    Ok(())
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

fn show_shell_window(window: HWND) -> Result<(), ShellError> {
    // SAFETY: All callers supply a live top-level HWND owned by this UI thread.
    let _ = unsafe { ShowWindow(window, SW_SHOWDEFAULT) };
    let pointer = state_pointer_for(window);
    // SAFETY: ShowWindow does not detach the parent-owned state.
    let preference = unsafe { pointer.as_ref() }
        .ok_or_else(|| ShellError::new("window state is unavailable during first presentation"))?
        .theme
        .preference;
    // Reapply after every child is visible and laid out. Hidden controls can
    // discard their initial invalidation, which previously left a fresh
    // System-dark status strip painted with the class's light background.
    refresh_theme(window, preference)?;
    // SAFETY: The refreshed parent and child update regions are painted before
    // startup returns to the message loop.
    let _ = unsafe { UpdateWindow(window) };
    Ok(())
}

fn post_initial_presentation(window: HWND) -> Result<(), ShellError> {
    // SAFETY: This posts a pointer-free private message to the live UI window.
    unsafe { PostMessageW(Some(window), WM_APP_PRESENT_SHELL, WPARAM(0), LPARAM(0)) }
        .map_err(|error| ShellError::new(format!("initial presentation post failed: {error}")))
}

fn handle_initial_presentation(window: HWND) -> LRESULT {
    if let Err(error) = show_shell_window(window) {
        show_startup_error(&error.to_string());
        let _ = destroy_shell_window(window);
    }
    LRESULT(0)
}

fn load_accelerators(instance: HINSTANCE) -> Result<AcceleratorGuard, ShellError> {
    let resource = PCWSTR(std::ptr::without_provenance::<u16>(RESOURCE_ACCELERATOR_ID));
    // SAFETY: Resource 102 is compiled into this module as an ACCELERATORS table.
    // The guard releases the returned handle after the message loop exits.
    unsafe { LoadAcceleratorsW(Some(instance), resource) }
        .map(AcceleratorGuard)
        .map_err(|error| ShellError::new(format!("embedded accelerator load failed: {error}")))
}

fn show_overflow_menu(window: HWND) -> Result<(), ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: Commands run synchronously while the UI-owned state is attached.
    let state = unsafe { pointer.as_ref() }
        .ok_or_else(|| ShellError::new("window state is unavailable"))?;
    // SAFETY: The popup handle is newly allocated and stays under this guard.
    let menu = MenuGuard::new(unsafe { CreatePopupMenu() }.map_err(menu_error)?);
    append_menu_text(menu.handle(), ID_FILE_OPEN, w!("&Open...\tCtrl+O"))?;
    append_menu_text(menu.handle(), ID_FILE_RELOAD, w!("&Reload\tF5"))?;
    append_menu_item(
        menu.handle(),
        ID_TAB_CLOSE,
        w!("Close &tab\tCtrl+W"),
        state.current_path.is_some() || state.tab_order.len() > 1,
    )?;
    append_menu_item(
        menu.handle(),
        ID_TAB_NEXT,
        w!("Next ta&b\tCtrl+Tab"),
        state.tab_order.len() > 1,
    )?;
    // SAFETY: The guarded popup is mutable until TrackPopupMenuEx returns.
    unsafe { AppendMenuW(menu.handle(), MF_SEPARATOR, 0, None) }.map_err(menu_error)?;
    append_menu_text(
        menu.handle(),
        ID_EDIT_COPY,
        w!("&Copy selected rows\tCtrl+C"),
    )?;
    append_menu_text(menu.handle(), ID_EDIT_FIND, w!("Focus &Find\tCtrl+F"))?;
    append_menu_text(menu.handle(), ID_EDIT_FIND_NEXT, w!("Find &next\tF3"))?;
    append_menu_text(
        menu.handle(),
        ID_EDIT_FIND_PREVIOUS,
        w!("Find &previous\tShift+F3"),
    )?;
    append_menu_text(menu.handle(), ID_EDIT_GOTO, w!("&Go to row...\tCtrl+G"))?;
    let case_flag = if inline_match_case(state.chrome.match_case) {
        MF_STRING | MF_CHECKED
    } else {
        MF_STRING | MF_UNCHECKED
    };
    // SAFETY: The popup and static label remain valid for this synchronous call.
    unsafe {
        AppendMenuW(
            menu.handle(),
            case_flag,
            usize::from(ID_SEARCH_MATCH_CASE),
            w!("Match &case"),
        )
    }
    .map_err(menu_error)?;
    unsafe { AppendMenuW(menu.handle(), MF_SEPARATOR, 0, None) }.map_err(menu_error)?;
    let theme_label = format!("Theme: {}", state.theme.preference.label());
    append_menu_owned(menu.handle(), ID_VIEW_THEME, &theme_label)?;
    append_menu_text(menu.handle(), ID_HELP_ABOUT, w!("&About LeanRows"))?;
    unsafe { AppendMenuW(menu.handle(), MF_SEPARATOR, 0, None) }.map_err(menu_error)?;
    append_menu_text(menu.handle(), ID_FILE_EXIT, w!("E&xit"))?;

    let mut anchor = RECT::default();
    // SAFETY: The overflow button belongs to this window and anchor is writable.
    unsafe { GetWindowRect(state.chrome.more, &raw mut anchor) }
        .map_err(|error| ShellError::new(format!("overflow anchor lookup failed: {error}")))?;
    // SAFETY: Foreground activation and popup tracking occur on this UI thread.
    let _ = unsafe { SetForegroundWindow(window) };
    let selected = unsafe {
        TrackPopupMenuEx(
            menu.handle(),
            (TPM_RETURNCMD | TPM_RIGHTALIGN).0,
            anchor.right,
            anchor.bottom,
            window,
            None,
        )
    };
    if let Ok(command) = u16::try_from(selected.0)
        && command != 0
    {
        handle_command(window, command);
    }
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

fn append_menu_item(
    menu: HMENU,
    command: u16,
    text: PCWSTR,
    enabled: bool,
) -> Result<(), ShellError> {
    let flags = if enabled {
        MF_STRING
    } else {
        MF_STRING | MF_GRAYED
    };
    // SAFETY: The caller owns the menu and supplies a live NUL-terminated label.
    unsafe { AppendMenuW(menu, flags, usize::from(command), text) }.map_err(menu_error)
}

fn append_menu_owned(menu: HMENU, command: u16, text: &str) -> Result<(), ShellError> {
    let mut encoded: Vec<u16> = text.encode_utf16().filter(|unit| *unit != 0).collect();
    encoded.push(0);
    append_menu_text(menu, command, PCWSTR(encoded.as_ptr()))
}

fn handle_command(window: HWND, command: u16) {
    match command {
        ID_FILE_OPEN => match choose_files(window) {
            Ok(paths) => open_documents(window, paths),
            Err(error) => set_status(window, &error.to_string()),
        },
        ID_TAB_CLOSE => {
            if let Err(error) = close_active_tab(window) {
                set_status(window, &format!("Close tab failed: {error}"));
            }
        }
        ID_TAB_NEXT => switch_tab(window, true),
        ID_TAB_PREVIOUS => switch_tab(window, false),
        ID_TAB_SELECT_FIRST..=ID_TAB_SELECT_LAST => select_tab_number(window, command),
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
        ID_SEARCH_MATCH_CASE => toggle_match_case(window),
        ID_VIEW_THEME => {
            if let Err(error) = cycle_theme(window) {
                set_status(window, &format!("Theme change failed: {error}"));
            }
        }
        ID_APP_MORE => {
            if let Err(error) = show_overflow_menu(window) {
                set_status(window, &format!("Menu failed: {error}"));
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

fn toggle_match_case(window: HWND) {
    let pointer = state_pointer_for(window);
    // SAFETY: The checkbox belongs to this UI thread and is queried synchronously.
    let Some(checkbox) = (unsafe { pointer.as_ref() }).map(|state| state.chrome.match_case) else {
        return;
    };
    let next = if inline_match_case(checkbox) {
        WPARAM(0)
    } else {
        WPARAM(usize::try_from(BST_CHECKED.0).unwrap_or_default())
    };
    // SAFETY: BM_SETCHECK carries only the documented small integer state.
    unsafe { SendMessageW(checkbox, BM_SETCHECK, Some(next), Some(LPARAM(0))) };
}

fn cycle_theme(window: HWND) -> Result<(), ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: Theme changes are serialized on the UI thread.
    let state = unsafe { pointer.as_ref() }
        .ok_or_else(|| ShellError::new("window state is unavailable"))?;
    refresh_theme(window, state.theme.preference.next())
}

fn refresh_theme(window: HWND, preference: ThemeMode) -> Result<(), ShellError> {
    // SAFETY: The live HWND provides its current monitor DPI.
    let dpi = unsafe { GetDpiForWindow(window) }.max(96);
    let next = ThemeResources::new(preference, dpi)?;
    let pointer = state_pointer_for(window);
    // SAFETY: Theme refresh runs on the window's UI thread.
    let state = unsafe { pointer.as_mut() }
        .ok_or_else(|| ShellError::new("window state is unavailable"))?;
    let previous = std::mem::replace(&mut state.theme, next);
    let result = apply_native_presentation(state);
    drop(previous);
    result?;
    let _ = preference.save();
    layout_children(window);
    redraw_shell_children(window)
}

fn redraw_shell_children(window: HWND) -> Result<(), ShellError> {
    // SAFETY: The UI-thread-owned window and every child must repaint from the
    // current theme after preference changes or worker-driven text updates.
    unsafe {
        RedrawWindow(
            Some(window),
            None,
            None,
            RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW,
        )
    }
    .as_bool()
    .then_some(())
    .ok_or_else(|| ShellError::new("native child surfaces could not be repainted"))
}

fn show_find_dialog(window: HWND) -> Result<(), ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: Commands are dispatched synchronously on the window's UI thread.
    let state = unsafe { pointer.as_mut() }
        .ok_or_else(|| ShellError::new("window state is unavailable"))?;
    if state.current_path.is_none() || state.active_serial.is_none() {
        return Err(ShellError::new("open a document before searching"));
    }
    if state.chrome.search_edit.0.is_null() {
        return Err(ShellError::new("inline Find control is unavailable"));
    }
    // SAFETY: The edit control belongs to this UI thread. Selecting the full
    // bounded value mirrors LeanMark's Ctrl+F behavior without allocating.
    unsafe {
        let _ = SetFocus(Some(state.chrome.search_edit));
        SendMessageW(
            state.chrome.search_edit,
            EM_SETSEL,
            Some(WPARAM(0)),
            Some(LPARAM(-1)),
        );
    }
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
    let needle = inline_find_needle(state.chrome.search_edit)?;
    let case_sensitive = inline_match_case(state.chrome.match_case);
    let status = start_or_navigate_find(state, needle, case_sensitive, direction)?;
    set_status_handle(state.status, &status);
    Ok(())
}

fn inline_find_needle(edit: HWND) -> Result<Vec<u8>, ShellError> {
    // SAFETY: This is a read-only length query for a live single-line edit.
    let length = unsafe { GetWindowTextLengthW(edit) };
    let length = usize::try_from(length.max(0))
        .map_err(|_| ShellError::new("Find input length exceeds the native limit"))?;
    if length >= MAX_FIND_UTF16_UNITS {
        return Err(ShellError::new("Find text is too long"));
    }
    let mut buffer = vec![0_u16; length.saturating_add(1)];
    // SAFETY: The writable slice includes one unit for the terminating NUL.
    let copied = unsafe { GetWindowTextW(edit, &mut buffer) };
    let copied = usize::try_from(copied.max(0))
        .map_err(|_| ShellError::new("Find input could not be read"))?;
    find_needle(&buffer[..copied.min(buffer.len())])
}

fn inline_match_case(button: HWND) -> bool {
    // SAFETY: BM_GETCHECK carries no pointers and targets a live checkbox.
    unsafe { SendMessageW(button, BM_GETCHECK, None, None) }.0
        == isize::try_from(BST_CHECKED.0).unwrap_or_default()
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

fn choose_files(window: HWND) -> Result<Vec<PathBuf>, ShellError> {
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
        Flags: OFN_EXPLORER
            | OFN_FILEMUSTEXIST
            | OFN_PATHMUSTEXIST
            | OFN_HIDEREADONLY
            | OFN_ALLOWMULTISELECT,
        ..Default::default()
    };
    // SAFETY: The dialog owns no borrowed data after return; the path buffer is writable.
    if unsafe { GetOpenFileNameW(&raw mut dialog) }.as_bool() {
        return Ok(parse_open_selection(&buffer));
    }
    // SAFETY: This immediately queries the thread-local extended dialog result.
    let code = unsafe { CommDlgExtendedError() };
    if code.0 == 0 {
        Ok(Vec::new())
    } else if code == FNERR_BUFFERTOOSMALL {
        Err(ShellError::new(
            "Too many files were selected at once; select fewer files",
        ))
    } else {
        Err(ShellError::new(format!(
            "Open dialog failed with code 0x{:04X}",
            code.0
        )))
    }
}

/// Reads an Explorer-style multi-select result: one full path, or a folder
/// followed by file names, each NUL-terminated and ending with an empty entry.
fn parse_open_selection(buffer: &[u16]) -> Vec<PathBuf> {
    let mut parts = buffer
        .split(|unit| *unit == 0)
        .take_while(|part| !part.is_empty());
    let Some(first) = parts.next() else {
        return Vec::new();
    };
    let first = PathBuf::from(OsString::from_wide(first));
    let names: Vec<PathBuf> = parts
        .map(|name| first.join(OsString::from_wide(name)))
        .collect();
    if names.is_empty() { vec![first] } else { names }
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
    let _ = set_control_text(status, text);
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
        lpszClassName: WINDOW_CLASS_NAME,
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

fn apply_native_presentation(state: &WindowState) -> Result<(), ShellError> {
    // SAFETY: The current executable module stays loaded for the process lifetime.
    let module = unsafe { GetModuleHandleW(None) }
        .map_err(|error| ShellError::new(format!("module lookup failed: {error}")))?;
    let icon = load_resource_icon(
        HINSTANCE(module.0),
        state.theme.metrics.wordmark_size,
        state.theme.metrics.wordmark_size,
    )?;
    // SAFETY: LR_SHARED leaves icon ownership with the module/system.
    unsafe {
        SendMessageW(
            state.chrome.wordmark,
            STM_SETICON,
            Some(WPARAM(icon.0.addr())),
            Some(LPARAM(0)),
        );
    }
    let body = state.theme.fonts.body();
    let semibold = state.theme.fonts.semibold();
    let caption = state.theme.fonts.caption();
    let heading = state.theme.fonts.heading();
    for handle in [
        state.list,
        state.chrome.search_edit,
        state.chrome.match_case,
        state.chrome.find_previous,
        state.chrome.find_next,
        state.chrome.reload,
        state.chrome.goto,
        state.chrome.theme,
        state.chrome.open,
        state.chrome.more,
        state.chrome.tab_strip,
        state.chrome.empty_body,
        state.chrome.empty_open,
    ] {
        apply_font(handle, body);
    }
    for handle in [state.chrome.wordmark, state.chrome.file_name] {
        apply_font(handle, semibold);
    }
    if let Some(header) = list_header(state.list) {
        apply_font(header, semibold);
    }
    for handle in [
        state.chrome.file_meta,
        state.chrome.empty_eyebrow,
        state.status,
    ] {
        apply_font(handle, caption);
    }
    apply_font(state.chrome.empty_title, heading);

    let palette = state.theme.palette;
    // SAFETY: List-view color messages are synchronous integer payloads.
    unsafe {
        SendMessageW(
            state.list,
            LVM_SETBKCOLOR,
            Some(WPARAM(0)),
            Some(LPARAM(
                isize::try_from(palette.surface.colorref().0).unwrap_or_default(),
            )),
        );
        SendMessageW(
            state.list,
            LVM_SETTEXTBKCOLOR,
            Some(WPARAM(0)),
            Some(LPARAM(
                isize::try_from(palette.surface.colorref().0).unwrap_or_default(),
            )),
        );
        SendMessageW(
            state.list,
            LVM_SETTEXTCOLOR,
            Some(WPARAM(0)),
            Some(LPARAM(
                isize::try_from(palette.text_primary.colorref().0).unwrap_or_default(),
            )),
        );
    }
    let visual_class = if matches!(state.theme.effective, EffectiveTheme::Light) {
        w!("Explorer")
    } else {
        w!("")
    };
    let visual_subclass = if matches!(state.theme.effective, EffectiveTheme::Light) {
        PCWSTR::null()
    } else {
        w!("")
    };
    // SAFETY: These controls are live on this UI thread. An empty class/subclass
    // pair is the documented way to disable visual-style painting so the app's
    // explicit dark/high-contrast colors are not overwritten by light chrome.
    unsafe {
        let _ = SetWindowTheme(state.list, visual_class, visual_subclass);
        let _ = SetWindowTheme(state.chrome.search_edit, visual_class, visual_subclass);
        let _ = SetWindowTheme(state.chrome.match_case, visual_class, visual_subclass);
    }
    let _ = state.theme.apply_window_chrome(state.window);
    set_control_text(state.chrome.theme, state.theme.preference.label())?;
    invalidate_native_presentation(state);
    Ok(())
}

fn invalidate_native_presentation(state: &WindowState) {
    for handle in [
        state.window,
        state.list,
        state.status,
        state.chrome.topbar,
        state.chrome.file_name,
        state.chrome.file_meta,
        state.chrome.search_edit,
        state.chrome.match_case,
        state.chrome.find_previous,
        state.chrome.find_next,
        state.chrome.reload,
        state.chrome.goto,
        state.chrome.theme,
        state.chrome.open,
        state.chrome.more,
        state.chrome.empty_eyebrow,
        state.chrome.empty_title,
        state.chrome.empty_body,
        state.chrome.empty_open,
        state.chrome.status_backdrop,
    ] {
        if !handle.0.is_null() {
            // SAFETY: Every HWND is owned by this UI thread and repaints after
            // the new theme resources have replaced the previous resource set.
            let _ = unsafe { InvalidateRect(Some(handle), None, true) };
        }
    }
    if let Some(header) = list_header(state.list) {
        // SAFETY: The header is owned by the live list view.
        let _ = unsafe { InvalidateRect(Some(header), None, false) };
    }
    progress::invalidate(state.chrome.progress);
    tabs::invalidate(state.chrome.tab_strip);
}

fn apply_font(handle: HWND, font: windows::Win32::Graphics::Gdi::HFONT) {
    if handle.0.is_null() {
        return;
    }
    // SAFETY: ThemeResources owns the font longer than every child control.
    unsafe {
        SendMessageW(
            handle,
            WM_SETFONT,
            Some(WPARAM(font.0.addr())),
            Some(LPARAM(1)),
        );
    }
}

fn list_header(list: HWND) -> Option<HWND> {
    // SAFETY: This synchronous query returns the list view's owned header HWND.
    let raw = unsafe { SendMessageW(list, LVM_GETHEADER, None, None) }.0;
    (raw != 0).then_some(HWND(raw as *mut c_void))
}

fn apply_shell_visibility(state: &WindowState) {
    for handle in [
        state.chrome.topbar,
        state.chrome.status_backdrop,
        state.status,
    ] {
        set_control_visible(handle, true);
    }
    set_control_visible(state.list, state.grid_visible);
    for handle in [
        state.chrome.empty_eyebrow,
        state.chrome.empty_title,
        state.chrome.empty_body,
        state.chrome.empty_open,
    ] {
        set_control_visible(handle, !state.grid_visible);
    }
    set_control_visible(state.chrome.progress, state.progress_visible);
}

fn set_control_visible(handle: HWND, visible: bool) {
    if handle.0.is_null() {
        return;
    }
    // SAFETY: The handle belongs to this UI thread and remains live.
    let _ = unsafe { ShowWindow(handle, if visible { SW_SHOW } else { SW_HIDE }) };
}

fn set_control_text(handle: HWND, text: &str) -> Result<(), ShellError> {
    let mut encoded: Vec<u16> = text.encode_utf16().filter(|unit| *unit != 0).collect();
    encoded.push(0);
    // SAFETY: The UTF-16 buffer is retained for the synchronous text copy.
    unsafe { SetWindowTextW(handle, PCWSTR(encoded.as_ptr())) }
        .map_err(|error| ShellError::new(format!("control text update failed: {error}")))
}

fn verify_native_presentation(window: HWND, state: &WindowState) -> Result<(), ShellError> {
    // SAFETY: These calls query class-owned icons and child-control fonts only.
    let icons_present = unsafe { GetClassLongPtrW(window, GCLP_HICON) } != 0
        && unsafe { GetClassLongPtrW(window, GCLP_HICONSM) } != 0;
    let fonts_present = unsafe { SendMessageW(state.list, WM_GETFONT, None, None) }.0 != 0
        && unsafe { SendMessageW(state.status, WM_GETFONT, None, None) }.0 != 0
        && unsafe { SendMessageW(state.chrome.search_edit, WM_GETFONT, None, None) }.0 != 0
        && unsafe { SendMessageW(state.chrome.empty_title, WM_GETFONT, None, None) }.0 != 0;
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
        if handle_shell_keyboard(window, &message) {
            continue;
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

fn handle_shell_keyboard(window: HWND, message: &MSG) -> bool {
    if message.message != WM_KEYDOWN {
        return false;
    }
    let pointer = state_pointer_for(window);
    // SAFETY: Message dispatch and state access remain serialized on this UI thread.
    let Some(state) = (unsafe { pointer.as_ref() }) else {
        return false;
    };
    let key = u16::try_from(message.wParam.0).unwrap_or_default();
    if message.hwnd == state.chrome.search_edit && key == VK_RETURN.0 {
        // SAFETY: This is a read-only query of the current keyboard state.
        let direction = if unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0 {
            QueryDirection::Previous
        } else {
            QueryDirection::Next
        };
        if let Err(error) = repeat_find(window, direction) {
            set_status(window, &format!("Find failed: {error}"));
        }
        return true;
    }
    if message.hwnd == state.chrome.search_edit && key == VK_ESCAPE.0 {
        // SAFETY: Focus remains within controls owned by this UI thread.
        let _ = unsafe { SetFocus(Some(state.list)) };
        return true;
    }
    if key != VK_TAB.0 {
        return false;
    }
    // SAFETY: This is a read-only query of the current keyboard state.
    if unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } < 0 {
        // Ctrl+Tab switches files through the accelerator table.
        return false;
    }
    focus_adjacent_control(state);
    true
}

fn focus_adjacent_control(state: &WindowState) {
    let controls = [
        state.chrome.open,
        state.chrome.search_edit,
        state.chrome.match_case,
        state.chrome.find_previous,
        state.chrome.find_next,
        state.chrome.theme,
        state.chrome.more,
        state.chrome.tab_strip,
        state.chrome.empty_open,
        state.list,
    ];
    // SAFETY: These calls query or move focus only among UI-thread-owned controls.
    let current = unsafe { GetFocus() };
    let reverse = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0;
    let start = controls
        .iter()
        .position(|control| *control == current)
        .unwrap_or(controls.len().saturating_sub(1));
    for offset in 1..=controls.len() {
        let index = if reverse {
            start.wrapping_add(controls.len()).wrapping_sub(offset) % controls.len()
        } else {
            start.saturating_add(offset) % controls.len()
        };
        let candidate = controls[index];
        if !candidate.0.is_null()
            && unsafe { IsWindowVisible(candidate) }.as_bool()
            && unsafe { IsWindowEnabled(candidate) }.as_bool()
        {
            let _ = unsafe { SetFocus(Some(candidate)) };
            return;
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
            handle_size(window, wparam);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            apply_minimum_window_size(window, lparam);
            LRESULT(0)
        }
        WM_ERASEBKGND => paint_window_background(window, wparam),
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORBTN => {
            control_color(window, wparam, lparam)
        }
        WM_DRAWITEM => {
            if draw_shell_surface(window, lparam) || draw_shell_button(window, lparam) {
                LRESULT(1)
            } else {
                // SAFETY: Unhandled owner-draw messages retain default processing.
                unsafe { DefWindowProcW(window, message, wparam, lparam) }
            }
        }
        WM_DROPFILES => {
            handle_drop(window, wparam.0);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            handle_dpi_changed(window, lparam);
            LRESULT(0)
        }
        WM_THEMECHANGED | WM_SETTINGCHANGE | WM_SYSCOLORCHANGE => {
            refresh_current_theme(window);
            LRESULT(0)
        }
        WM_NOTIFY => handle_notify(window, lparam),
        WM_COPYDATA => LRESULT(isize::from(receive_open_request(window, lparam))),
        WM_COMMAND => {
            if let Ok(command) = u16::try_from(wparam.0 & 0xffff)
                && (command != ID_SEARCH_MATCH_CASE || lparam.0 == 0)
            {
                handle_command(window, command);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = destroy_shell_window(window);
            LRESULT(0)
        }
        WM_APP_WORKER_READY => {
            apply_worker_event(window, wparam.0);
            LRESULT(0)
        }
        WM_APP_PRESENT_SHELL => handle_initial_presentation(window),
        WM_APP_OPEN_PENDING | WM_APP_CLOSE_TAB => {
            handle_tab_message(window, message, wparam);
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
            handle_destroy(window);
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

fn handle_notify(window: HWND, lparam: LPARAM) -> LRESULT {
    if handle_tab_notification(window, lparam) {
        return LRESULT(0);
    }
    handle_custom_draw(window, lparam).unwrap_or_else(|| {
        handle_list_notification(window, lparam);
        LRESULT(0)
    })
}

fn handle_tab_message(window: HWND, message: u32, wparam: WPARAM) {
    if message == WM_APP_OPEN_PENDING {
        open_pending(window);
    } else if let Err(error) = close_tab(window, wparam.0) {
        set_status(window, &format!("Close tab failed: {error}"));
    }
}

fn handle_size(window: HWND, wparam: WPARAM) {
    if wparam.0 == SIZE_MINIMIZED as usize {
        // Nothing is visible while minimized, so hand memory back to Windows.
        process::trim_working_set();
    } else {
        layout_children(window);
    }
}

fn handle_destroy(window: HWND) {
    let pointer = state_pointer_for(window);
    // SAFETY: The state remains attached through WM_NCDESTROY.
    let workers = unsafe { pointer.as_mut() }.map(|state| {
        state.closing = true;
        let _ = state.clear_accessibility();
        (
            state.worker.take(),
            std::mem::take(&mut state.background_tabs),
        )
    });
    // Dropping each worker stops its thread and closes its file.
    drop(workers);
    // SAFETY: Standard termination for this thread's message loop.
    unsafe { PostQuitMessage(0) };
}

fn handle_dpi_changed(window: HWND, lparam: LPARAM) {
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
    refresh_current_theme(window);
}

fn refresh_current_theme(window: HWND) {
    let pointer = state_pointer_for(window);
    // SAFETY: Preference is copied while the attached state is live.
    if let Some(preference) = unsafe { pointer.as_ref() }.map(|state| state.theme.preference) {
        let _ = refresh_theme(window, preference);
    }
}

fn apply_minimum_window_size(window: HWND, lparam: LPARAM) {
    if lparam.0 == 0 {
        return;
    }
    // SAFETY: WM_GETMINMAXINFO supplies this writable pointer for the call.
    let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
    // SAFETY: The live HWND provides its current monitor DPI.
    let dpi = unsafe { GetDpiForWindow(window) }.max(96);
    let metrics = UiLayout::calculate(0, 0, dpi, false).metrics;
    let mut bounds = RECT {
        left: 0,
        top: 0,
        right: metrics.minimum_client_width,
        bottom: metrics.minimum_client_height,
    };
    // SAFETY: The RECT is writable and matches the live top-level style.
    if unsafe {
        AdjustWindowRectExForDpi(
            &raw mut bounds,
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            false,
            WS_EX_APPWINDOW | WS_EX_ACCEPTFILES,
            dpi,
        )
    }
    .is_ok()
    {
        info.ptMinTrackSize.x = bounds.right.saturating_sub(bounds.left);
        info.ptMinTrackSize.y = bounds.bottom.saturating_sub(bounds.top);
    }
}

fn paint_window_background(window: HWND, wparam: WPARAM) -> LRESULT {
    let pointer = state_pointer_for(window);
    // SAFETY: The state remains attached while window messages are dispatched.
    let Some(state) = (unsafe { pointer.as_ref() }) else {
        return LRESULT(0);
    };
    let mut client = RECT::default();
    // SAFETY: WM_ERASEBKGND supplies a live HDC and the RECT is writable.
    if unsafe { GetClientRect(window, &raw mut client) }.is_err() {
        return LRESULT(0);
    }
    let device = HDC(wparam.0 as *mut c_void);
    let painted = unsafe { FillRect(device, &raw const client, state.theme.brushes.canvas()) };
    LRESULT((painted != 0).into())
}

fn control_color(window: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let pointer = state_pointer_for(window);
    // SAFETY: State and child-control handles remain live during color messages.
    let Some(state) = (unsafe { pointer.as_ref() }) else {
        return LRESULT(0);
    };
    let child = HWND(lparam.0 as *mut c_void);
    let (brush, background, foreground) =
        if child == state.status || child == state.chrome.status_backdrop {
            (
                state.theme.brushes.surface_muted(),
                state.theme.palette.surface_muted,
                state.theme.palette.text_secondary,
            )
        } else if child == state.chrome.topbar
            || child == state.chrome.file_name
            || child == state.chrome.file_meta
            || child == state.chrome.open
            || child == state.chrome.more
            || child == state.chrome.theme
            || child == state.chrome.reload
            || child == state.chrome.goto
            || child == state.chrome.find_previous
            || child == state.chrome.find_next
            || child == state.chrome.match_case
        {
            (
                state.theme.brushes.surface(),
                state.theme.palette.surface,
                state.theme.palette.text_primary,
            )
        } else if child == state.chrome.wordmark {
            (
                state.theme.brushes.surface_muted(),
                state.theme.palette.surface_muted,
                state.theme.palette.accent,
            )
        } else if child == state.chrome.empty_eyebrow {
            (
                state.theme.brushes.canvas(),
                state.theme.palette.canvas,
                state.theme.palette.accent,
            )
        } else {
            (
                state.theme.brushes.canvas(),
                state.theme.palette.canvas,
                state.theme.palette.text_primary,
            )
        };
    let device = HDC(wparam.0 as *mut c_void);
    // SAFETY: The HDC is valid only for this synchronous control-color message.
    unsafe {
        let _ = SetBkColor(device, background.colorref());
        let _ = SetTextColor(device, foreground.colorref());
        let _ = SetBkMode(device, TRANSPARENT);
    }
    LRESULT(isize::try_from(brush.0.addr()).unwrap_or_default())
}

fn draw_shell_button(window: HWND, lparam: LPARAM) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    // SAFETY: WM_DRAWITEM supplies a live DRAWITEMSTRUCT for this synchronous call.
    let item = unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) };
    if item.CtlType != ODT_BUTTON {
        return false;
    }
    let pointer = state_pointer_for(window);
    // SAFETY: State and child handles remain attached throughout message dispatch.
    let Some(state) = (unsafe { pointer.as_ref() }) else {
        return false;
    };
    let is_primary = item.hwndItem == state.chrome.open || item.hwndItem == state.chrome.empty_open;
    let is_command = [
        state.chrome.find_previous,
        state.chrome.find_next,
        state.chrome.reload,
        state.chrome.goto,
        state.chrome.theme,
        state.chrome.more,
    ]
    .contains(&item.hwndItem);
    if !is_primary && !is_command {
        return false;
    }
    paint_shell_button(state, item, is_primary)
}

fn draw_shell_surface(window: HWND, lparam: LPARAM) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    // SAFETY: WM_DRAWITEM supplies a live DRAWITEMSTRUCT for this synchronous call.
    let item = unsafe { &*(lparam.0 as *const DRAWITEMSTRUCT) };
    if item.CtlType != ODT_STATIC {
        return false;
    }
    let pointer = state_pointer_for(window);
    // SAFETY: State and child handles remain attached throughout message dispatch.
    let Some(state) = (unsafe { pointer.as_ref() }) else {
        return false;
    };
    let brush = if item.hwndItem == state.chrome.topbar {
        state.theme.brushes.surface()
    } else if item.hwndItem == state.chrome.status_backdrop || item.hwndItem == state.status {
        state.theme.brushes.surface_muted()
    } else {
        return false;
    };
    // SAFETY: The control-provided HDC, bounds, and theme brush are live here.
    unsafe {
        let _ = FillRect(item.hDC, &raw const item.rcItem, brush);
    }
    if item.hwndItem == state.status {
        paint_status_text(state, item);
    }
    true
}

fn paint_status_text(state: &WindowState, item: &DRAWITEMSTRUCT) {
    let mut text = [0_u16; STATUS_PAINT_TEXT_UNITS];
    // SAFETY: The status HWND is live and the fixed local buffer is writable.
    let length = unsafe { GetWindowTextW(state.status, &mut text) };
    let length = usize::try_from(length).unwrap_or_default().min(text.len());
    // SAFETY: The HDC belongs to this draw callback; theme resources outlive it.
    unsafe {
        let previous_font = SelectObject(item.hDC, HGDIOBJ(state.theme.fonts.caption().0));
        let _ = SetBkMode(item.hDC, TRANSPARENT);
        let _ = SetTextColor(item.hDC, state.theme.palette.text_secondary.colorref());
        let mut bounds = item.rcItem;
        let _ = DrawTextW(
            item.hDC,
            &mut text[..length],
            &raw mut bounds,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        if !previous_font.is_invalid() {
            let _ = SelectObject(item.hDC, previous_font);
        }
    }
}

fn paint_shell_button(state: &WindowState, item: &DRAWITEMSTRUCT, is_primary: bool) -> bool {
    let background = if item.hwndItem == state.chrome.empty_open {
        state.theme.brushes.canvas()
    } else {
        state.theme.brushes.surface()
    };

    let disabled = item.itemState.0 & ODS_DISABLED.0 != 0;
    let selected = item.itemState.0 & ODS_SELECTED.0 != 0;
    let brush = if disabled {
        state.theme.brushes.surface_muted()
    } else if is_primary {
        state.theme.brushes.accent()
    } else if selected {
        state.theme.brushes.surface_muted()
    } else {
        state.theme.brushes.surface()
    };
    let foreground = if disabled {
        state.theme.palette.text_disabled
    } else if is_primary {
        state.theme.palette.accent_text
    } else {
        state.theme.palette.text_primary
    };
    let radius = state.theme.metrics.radius_small.saturating_mul(2);
    let region = unsafe {
        CreateRoundRectRgn(
            item.rcItem.left,
            item.rcItem.top,
            item.rcItem.right,
            item.rcItem.bottom,
            radius,
            radius,
        )
    };
    if region.is_invalid() {
        return false;
    }
    // SAFETY: The region, HDC, brush, and font remain valid for this draw callback.
    unsafe {
        let _ = FillRect(item.hDC, &raw const item.rcItem, background);
        let _ = FillRgn(item.hDC, region, brush);
        let border_width = state.theme.metrics.border_width.max(1);
        let _ = FrameRgn(
            item.hDC,
            region,
            state.theme.brushes.border(),
            border_width,
            border_width,
        );
        let _ = DeleteObject(HGDIOBJ(region.0));
        let previous_font = SelectObject(item.hDC, HGDIOBJ(state.theme.fonts.semibold().0));
        let _ = SetBkMode(item.hDC, TRANSPARENT);
        let _ = SetTextColor(item.hDC, foreground.colorref());
        let mut bounds = item.rcItem;
        if selected {
            let offset = state.theme.metrics.border_width.max(1);
            bounds.left = bounds.left.saturating_add(offset);
            bounds.top = bounds.top.saturating_add(offset);
            bounds.right = bounds.right.saturating_add(offset);
            bounds.bottom = bounds.bottom.saturating_add(offset);
        }
        let mut text = [0_u16; 64];
        let length = GetWindowTextW(item.hwndItem, &mut text);
        let length = usize::try_from(length).unwrap_or_default().min(text.len());
        let _ = DrawTextW(
            item.hDC,
            &mut text[..length],
            &raw mut bounds,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        if !previous_font.is_invalid() {
            let _ = SelectObject(item.hDC, previous_font);
        }
        if item.itemState.0 & ODS_FOCUS.0 != 0 && item.itemState.0 & ODS_NOFOCUSRECT.0 == 0 {
            let inset = state.theme.metrics.space_1.max(2);
            let focus = RECT {
                left: item.rcItem.left.saturating_add(inset),
                top: item.rcItem.top.saturating_add(inset),
                right: item.rcItem.right.saturating_sub(inset),
                bottom: item.rcItem.bottom.saturating_sub(inset),
            };
            let _ = DrawFocusRect(item.hDC, &raw const focus);
        }
    }
    true
}

fn handle_custom_draw(window: HWND, lparam: LPARAM) -> Option<LRESULT> {
    if lparam.0 == 0 {
        return None;
    }
    let pointer = state_pointer_for(window);
    // SAFETY: Every WM_NOTIFY payload starts with a readable NMHDR and the
    // attached state remains alive throughout this synchronous callback.
    let (Some(state), header) = (unsafe { pointer.as_ref() }, unsafe {
        &*(lparam.0 as *const NMHDR)
    }) else {
        return None;
    };
    if header.hwndFrom != state.chrome.match_case || header.code != NM_CUSTOMDRAW {
        return None;
    }
    // SAFETY: NM_CUSTOMDRAW identifies the exact payload type.
    let custom = unsafe { &*(lparam.0 as *const NMCUSTOMDRAW) };
    if custom.dwDrawStage != CDDS_PREPAINT || !draw_match_case(state, custom) {
        return None;
    }
    Some(LRESULT(
        isize::try_from(CDRF_SKIPDEFAULT).unwrap_or_default(),
    ))
}

fn draw_match_case(state: &WindowState, custom: &NMCUSTOMDRAW) -> bool {
    let bounds = custom.rc;
    let height = bounds.bottom.saturating_sub(bounds.top);
    if height <= 0 || bounds.right <= bounds.left {
        return false;
    }
    let disabled = custom.uItemState.contains(CDIS_DISABLED);
    let hot = custom.uItemState.contains(CDIS_HOT);
    let background = if hot {
        state.theme.brushes.surface_muted()
    } else {
        state.theme.brushes.surface()
    };
    let foreground = if disabled {
        state.theme.palette.text_disabled
    } else {
        state.theme.palette.text_primary
    };
    let box_size = state.theme.metrics.icon_small.min(height).max(1);
    let box_left = bounds.left.saturating_add(state.theme.metrics.space_1);
    let box_top = bounds
        .top
        .saturating_add(height.saturating_sub(box_size) / 2);
    let check_bounds = RECT {
        left: box_left,
        top: box_top,
        right: box_left.saturating_add(box_size),
        bottom: box_top.saturating_add(box_size),
    };
    let radius = state.theme.metrics.radius_small.max(1);
    // SAFETY: The bounded rectangle defines a temporary region owned here.
    let region = unsafe {
        CreateRoundRectRgn(
            check_bounds.left,
            check_bounds.top,
            check_bounds.right,
            check_bounds.bottom,
            radius,
            radius,
        )
    };
    if region.is_invalid() {
        return false;
    }
    // SAFETY: The control-provided HDC and theme-owned brushes/fonts remain
    // valid for this synchronous custom-draw stage.
    unsafe {
        let _ = FillRect(custom.hdc, &raw const bounds, background);
        let _ = FillRgn(custom.hdc, region, state.theme.brushes.surface());
        let border_width = state.theme.metrics.border_width.max(1);
        let _ = FrameRgn(
            custom.hdc,
            region,
            state.theme.brushes.border(),
            border_width,
            border_width,
        );
        let _ = DeleteObject(HGDIOBJ(region.0));
        let previous_font = SelectObject(custom.hdc, HGDIOBJ(state.theme.fonts.body().0));
        let _ = SetBkMode(custom.hdc, TRANSPARENT);
        if inline_match_case(state.chrome.match_case) {
            let _ = SetTextColor(custom.hdc, state.theme.palette.accent.colorref());
            let mut check = [0x2713_u16];
            let mut check_text_bounds = check_bounds;
            let _ = DrawTextW(
                custom.hdc,
                &mut check,
                &raw mut check_text_bounds,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
        }
        let _ = SetTextColor(custom.hdc, foreground.colorref());
        let mut label = [0_u16; 64];
        let length = GetWindowTextW(state.chrome.match_case, &mut label);
        let length = usize::try_from(length).unwrap_or_default().min(label.len());
        let mut label_bounds = RECT {
            left: check_bounds
                .right
                .saturating_add(state.theme.metrics.space_2),
            top: bounds.top,
            right: bounds.right,
            bottom: bounds.bottom,
        };
        let _ = DrawTextW(
            custom.hdc,
            &mut label[..length],
            &raw mut label_bounds,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        if !previous_font.is_invalid() {
            let _ = SelectObject(custom.hdc, previous_font);
        }
        if custom.uItemState.contains(CDIS_FOCUS) {
            let inset = state.theme.metrics.space_1.max(2);
            let focus = RECT {
                left: bounds.left.saturating_add(inset),
                top: bounds.top.saturating_add(inset),
                right: bounds.right.saturating_sub(inset),
                bottom: bounds.bottom.saturating_sub(inset),
            };
            let _ = DrawFocusRect(custom.hdc, &raw const focus);
        }
    }
    true
}

#[allow(
    clippy::too_many_lines,
    reason = "one linear construction path keeps partial Win32 child ownership auditable"
)]
fn create_children(window: HWND) -> Result<(), ()> {
    // SAFETY: The module belongs to this process.
    let module = unsafe { GetModuleHandleW(None) }.map_err(|_| ())?;
    let instance = HINSTANCE(module.0);
    let pointer = state_pointer_for(window);
    // SAFETY: `WM_NCCREATE` attached the state before `WM_CREATE`.
    let state = unsafe { pointer.as_mut() }.ok_or(())?;
    state.window = window;
    let topbar = create_child(
        instance,
        window,
        w!("STATIC"),
        w!(""),
        WS_CHILD | static_style(SS_OWNERDRAW.0),
        None,
    )?;
    let wordmark = create_child(
        instance,
        window,
        w!("STATIC"),
        w!(""),
        WS_CHILD | static_style(SS_ICON.0 | SS_CENTERIMAGE.0),
        None,
    )?;
    let file_name = create_child(instance, window, w!("STATIC"), w!(""), WS_CHILD, None)?;
    let file_meta = create_child(instance, window, w!("STATIC"), w!(""), WS_CHILD, None)?;
    let search_edit = create_child(
        instance,
        window,
        w!("EDIT"),
        w!(""),
        WS_CHILD | WS_TABSTOP | WS_BORDER | control_style(ES_AUTOHSCROLL),
        Some(ID_SEARCH_FIELD),
    )?;
    let match_case = create_child(
        instance,
        window,
        w!("BUTTON"),
        w!("Match case"),
        WS_CHILD | WS_TABSTOP | control_style(BS_AUTOCHECKBOX),
        Some(ID_SEARCH_MATCH_CASE),
    )?;
    let find_previous = create_button(instance, window, w!("<"), ID_EDIT_FIND_PREVIOUS)?;
    let find_next = create_button(instance, window, w!(">"), ID_EDIT_FIND_NEXT)?;
    let reload = create_button(instance, window, w!("Reload"), ID_FILE_RELOAD)?;
    let goto = create_button(instance, window, w!("Go to row"), ID_EDIT_GOTO)?;
    let theme = create_button(instance, window, w!("Theme"), ID_VIEW_THEME)?;
    let open = create_child(
        instance,
        window,
        w!("BUTTON"),
        w!("Open file"),
        WS_CHILD | WS_TABSTOP | control_style(BS_OWNERDRAW),
        Some(ID_FILE_OPEN),
    )?;
    let more = create_button(instance, window, w!("..."), ID_APP_MORE)?;
    let tab_strip = create_child(
        instance,
        window,
        WC_TABCONTROLW,
        w!("Open files"),
        WS_CHILD | WS_TABSTOP | WS_CLIPSIBLINGS | WINDOW_STYLE(TCS_FIXEDWIDTH),
        Some(ID_TAB_STRIP),
    )?;
    let progress = create_child(instance, window, PROGRESS_CLASSW, w!(""), WS_CHILD, None)?;
    let list = create_child(
        instance,
        window,
        WC_LISTVIEWW,
        w!("Rows"),
        WS_CHILD | WS_TABSTOP | WINDOW_STYLE(LVS_REPORT | LVS_OWNERDATA | LVS_SHOWSELALWAYS),
        None,
    )?;
    let empty_eyebrow = create_child(
        instance,
        window,
        w!("STATIC"),
        w!(""),
        WS_CHILD | static_style(SS_CENTER.0),
        None,
    )?;
    let empty_title = create_child(
        instance,
        window,
        w!("STATIC"),
        w!(""),
        WS_CHILD | static_style(SS_CENTER.0),
        None,
    )?;
    let empty_body = create_child(
        instance,
        window,
        w!("STATIC"),
        w!(""),
        WS_CHILD | static_style(SS_CENTER.0),
        None,
    )?;
    let empty_open = create_child(
        instance,
        window,
        w!("BUTTON"),
        w!("Open a data file"),
        WS_CHILD | WS_TABSTOP | control_style(BS_OWNERDRAW),
        Some(ID_FILE_OPEN),
    )?;
    let status_backdrop = create_child(
        instance,
        window,
        w!("STATIC"),
        w!(""),
        WS_CHILD | static_style(SS_OWNERDRAW.0),
        None,
    )?;
    let status = create_child(
        instance,
        window,
        w!("STATIC"),
        w!(""),
        WS_CHILD | static_style(SS_OWNERDRAW.0 | SS_CENTERIMAGE.0),
        None,
    )?;

    state.list = list;
    state.status = status;
    state.chrome = ChromeHandles {
        topbar,
        wordmark,
        file_name,
        file_meta,
        search_edit,
        match_case,
        find_previous,
        find_next,
        reload,
        goto,
        theme,
        open,
        more,
        tab_strip,
        progress,
        empty_eyebrow,
        empty_title,
        empty_body,
        empty_open,
        status_backdrop,
    };
    // Workers start with their first file, so an empty window runs no
    // background thread.
    let count = state.rows.visible_rows();

    insert_column(list, 0, w!("Row"), 112)?;
    insert_existing_data_columns(list, state.columns)?;
    let style = LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER;
    let style_parameter = isize::try_from(style).unwrap_or_default();
    // SAFETY: These control messages are synchronous and use bounded values or
    // static UTF-16 storage.
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
            search_edit,
            EM_SETCUEBANNER,
            Some(WPARAM(1)),
            Some(LPARAM(w!("Find in file").as_ptr() as isize)),
        );
        SendMessageW(progress, PBM_SETRANGE32, Some(WPARAM(0)), Some(LPARAM(100)));
    }
    progress::install_subclass(progress, window).map_err(|_| ())?;
    let header = list_header(list).ok_or(())?;
    header::install_subclass(header, window).map_err(|_| ())?;
    tabs::install_subclass(tab_strip, window).map_err(|_| ())?;
    // The progress rule straddles the lower edge of the chrome. Keeping it
    // above its siblings stops the tab strip from painting over it.
    // SAFETY: Both windows are live children created on this UI thread.
    let _ = unsafe {
        SetWindowPos(
            progress,
            Some(HWND_TOP),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    };
    set_welcome_text(state).map_err(|_| ())?;
    apply_native_presentation(state).map_err(|_| ())?;
    apply_shell_visibility(state);
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

fn create_child(
    instance: HINSTANCE,
    parent: HWND,
    class: PCWSTR,
    text: PCWSTR,
    style: WINDOW_STYLE,
    identifier: Option<u16>,
) -> Result<HWND, ()> {
    let menu = identifier.map(|identifier| {
        HMENU(std::ptr::without_provenance_mut::<c_void>(usize::from(
            identifier,
        )))
    });
    // SAFETY: The parent, class, text, identifier, and module belong to this UI thread.
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

fn create_button(
    instance: HINSTANCE,
    parent: HWND,
    text: PCWSTR,
    identifier: u16,
) -> Result<HWND, ()> {
    create_child(
        instance,
        parent,
        w!("BUTTON"),
        text,
        WS_CHILD | WS_TABSTOP | control_style(BS_OWNERDRAW),
        Some(identifier),
    )
}

fn control_style(value: i32) -> WINDOW_STYLE {
    WINDOW_STYLE(u32::try_from(value).unwrap_or_default())
}

fn static_style(value: u32) -> WINDOW_STYLE {
    WINDOW_STYLE(value)
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
    if state.list.0.is_null() || state.status.0.is_null() {
        return;
    }
    let mut client = RECT::default();
    // SAFETY: The top-level window is live and the RECT is writable.
    if unsafe { GetClientRect(window, &raw mut client) }.is_err() {
        return;
    }
    // SAFETY: The HWND is live. Zero is normalized by UiLayout.
    let dpi = state.theme.metrics.dpi;
    let layout = UiLayout::calculate(
        (client.right - client.left).max(0),
        (client.bottom - client.top).max(0),
        dpi,
        state.tab_order.len() > 1,
    );
    move_control(state.chrome.topbar, layout.top_bar);
    place_tab_strip(state, layout.tab_strip, layout.metrics);
    move_control(state.chrome.progress, layout.progress_track);
    move_control(state.list, layout.grid);
    move_control(state.chrome.status_backdrop, layout.status_strip);
    let status_inset = layout.metrics.space_5.min(layout.status_strip.width / 2);
    move_control(
        state.status,
        UiRect {
            x: layout.status_strip.x.saturating_add(status_inset),
            y: layout.status_strip.y,
            width: layout
                .status_strip
                .width
                .saturating_sub(status_inset.saturating_mul(2)),
            height: layout.status_strip.height,
        },
    );

    let commands = layout.commands;
    place_optional(state.chrome.wordmark, commands.wordmark);
    place_file_identity(
        state.chrome.file_name,
        state.chrome.file_meta,
        commands.file_identity,
        layout.metrics.space_1,
    );
    place_search_controls(
        state.chrome.search_edit,
        state.chrome.match_case,
        commands.search_field,
        commands.mode,
        layout.metrics.space_2,
        scale(88, dpi),
    );
    place_optional(state.chrome.find_previous, commands.previous_match_button);
    place_optional(state.chrome.find_next, commands.next_match_button);
    place_optional(state.chrome.reload, commands.reload_button);
    place_optional(state.chrome.goto, commands.goto_button);
    place_optional(state.chrome.theme, commands.theme_button);
    move_and_show(state.chrome.open, commands.open_button);
    move_and_show(state.chrome.more, commands.overflow_button);
    layout_empty_state(state, layout.empty_state, dpi);
}

fn place_tab_strip(state: &WindowState, rect: Option<UiRect>, metrics: layout::UiMetrics) {
    let Some(rect) = rect else {
        set_control_visible(state.chrome.tab_strip, false);
        return;
    };
    tabs::set_item_size(
        state.chrome.tab_strip,
        tab_item_width(rect.width, state.tab_order.len(), metrics),
        rect.height.saturating_sub(metrics.space_1),
    );
    move_and_show(state.chrome.tab_strip, rect);
}

fn move_control(handle: HWND, rect: UiRect) {
    if handle.0.is_null() {
        return;
    }
    // SAFETY: The control is owned by this UI thread; extents are non-negative.
    let _ = unsafe { MoveWindow(handle, rect.x, rect.y, rect.width, rect.height, true) };
}

fn move_and_show(handle: HWND, rect: UiRect) {
    move_control(handle, rect);
    set_control_visible(handle, rect.width > 0 && rect.height > 0);
}

fn place_optional(handle: HWND, rect: Option<UiRect>) {
    if let Some(rect) = rect {
        move_and_show(handle, rect);
    } else {
        set_control_visible(handle, false);
    }
}

fn place_file_identity(file_name: HWND, file_meta: HWND, rect: Option<UiRect>, gap: i32) {
    let Some(rect) = rect else {
        set_control_visible(file_name, false);
        set_control_visible(file_meta, false);
        return;
    };
    let usable = rect.height.saturating_sub(gap);
    let name_height = usable.saturating_mul(5) / 9;
    move_and_show(
        file_name,
        UiRect {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: name_height,
        },
    );
    move_and_show(
        file_meta,
        UiRect {
            x: rect.x,
            y: rect.y.saturating_add(name_height).saturating_add(gap),
            width: rect.width,
            height: usable.saturating_sub(name_height),
        },
    );
}

fn place_search_controls(
    search: HWND,
    match_case: HWND,
    rect: UiRect,
    mode: CommandLayoutMode,
    gap: i32,
    desired_case_width: i32,
) {
    if matches!(mode, CommandLayoutMode::Narrow) {
        move_and_show(search, rect);
        set_control_visible(match_case, false);
        return;
    }
    let case_width = desired_case_width.min(rect.width / 3).max(0);
    let search_width = rect.width.saturating_sub(case_width).saturating_sub(gap);
    move_and_show(
        search,
        UiRect {
            width: search_width,
            ..rect
        },
    );
    move_and_show(
        match_case,
        UiRect {
            x: rect.x.saturating_add(search_width).saturating_add(gap),
            width: case_width,
            ..rect
        },
    );
}

fn layout_empty_state(state: &WindowState, bounds: UiRect, dpi: u32) {
    let width = bounds.width.min(scale(560, dpi));
    let left = bounds
        .x
        .saturating_add(bounds.width.saturating_sub(width) / 2);
    let eyebrow_height = scale(20, dpi);
    let title_height = scale(72, dpi);
    let body_height = scale(60, dpi);
    let button_width = scale(168, dpi);
    let button_height = scale(44, dpi);
    let gap_small = scale(12, dpi);
    let gap_large = scale(24, dpi);
    let total_height = eyebrow_height
        .saturating_add(gap_small)
        .saturating_add(title_height)
        .saturating_add(gap_small)
        .saturating_add(body_height)
        .saturating_add(gap_large)
        .saturating_add(button_height);
    let mut top = bounds
        .y
        .saturating_add(bounds.height.saturating_sub(total_height) / 2);
    move_control(
        state.chrome.empty_eyebrow,
        UiRect {
            x: left,
            y: top,
            width,
            height: eyebrow_height,
        },
    );
    top = top.saturating_add(eyebrow_height).saturating_add(gap_small);
    move_control(
        state.chrome.empty_title,
        UiRect {
            x: left,
            y: top,
            width,
            height: title_height,
        },
    );
    top = top.saturating_add(title_height).saturating_add(gap_small);
    move_control(
        state.chrome.empty_body,
        UiRect {
            x: left,
            y: top,
            width,
            height: body_height,
        },
    );
    top = top.saturating_add(body_height).saturating_add(gap_large);
    move_control(
        state.chrome.empty_open,
        UiRect {
            x: bounds
                .x
                .saturating_add(bounds.width.saturating_sub(button_width) / 2),
            y: top,
            width: button_width.min(bounds.width),
            height: button_height,
        },
    );
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

fn post_worker_ready(raw_window: usize, tab_id: usize) {
    // SAFETY: The raw value was captured from this process's HWND. Posting a
    // pointer-free wake message is valid even if shutdown has begun; failure is ignored.
    let window = HWND(raw_window as *mut c_void);
    let _ = unsafe { PostMessageW(Some(window), WM_APP_WORKER_READY, WPARAM(tab_id), LPARAM(0)) };
}

fn start_worker(window: HWND, tab_id: usize) -> Result<Worker, ShellError> {
    let raw_window = window.0 as usize;
    let wake_ui: Arc<dyn Fn() + Send + Sync> =
        Arc::new(move || post_worker_ready(raw_window, tab_id));
    Worker::start(wake_ui)
        .map_err(|error| ShellError::new(format!("document worker could not start: {error}")))
}

fn queue_path(window: HWND, path: PathBuf) -> Result<(), ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: Commands are handled only while the window state is attached.
    let Some(state) = (unsafe { pointer.as_mut() }) else {
        return Err(ShellError::new("window state is unavailable"));
    };
    if state.worker.is_none() {
        state.worker = Some(start_worker(window, state.tab_id)?);
    }
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
    state.grid_visible = false;
    state.progress_visible = true;
    set_file_identity(state, &path, None)?;
    set_empty_state(
        state,
        "OPENING",
        "Preparing the first rows.",
        "LeanRows is reading a bounded window from the file. The source stays unchanged.",
    )?;
    state.current_path = Some(path);
    state.active_serial = Some(serial);
    apply_shell_visibility(state);

    // SAFETY: These messages are synchronous and carry only bounded integers.
    unsafe {
        SendMessageW(
            state.list,
            LVM_SETITEMCOUNT,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        );
    }
    progress::update_position(state.chrome.progress, 0);
    set_status_handle(state.status, "Opening file...");
    Ok(())
}

/// Shows `path` in a tab: the tab that already has it, the empty tab, or a
/// new tab to the right of the active one.
fn open_document(window: HWND, path: PathBuf) -> Result<(), ShellError> {
    let path = std::path::absolute(&path).unwrap_or(path);
    let pointer = state_pointer_for(window);
    // SAFETY: Commands and posted messages run on the UI thread that owns the state.
    let state = unsafe { pointer.as_mut() }
        .ok_or_else(|| ShellError::new("window state is unavailable"))?;
    if state
        .current_path
        .as_deref()
        .is_some_and(|open| process::same_path(open, &path))
    {
        return Ok(());
    }
    if let Some(id) = state
        .background_tabs
        .iter()
        .find(|tab| {
            tab.path
                .as_deref()
                .is_some_and(|open| process::same_path(open, &path))
        })
        .map(|tab| tab.id)
    {
        return activate_tab(window, id);
    }
    let mut new_tab = None;
    if state.current_path.is_some() {
        if state.tab_order.len() >= MAX_OPEN_TABS {
            return Err(ShellError::new(format!(
                "close a tab first; LeanRows keeps at most {MAX_OPEN_TABS} files open"
            )));
        }
        let outgoing = stash_active(state)?;
        state.background_tabs.push(outgoing);
        let id = state.next_tab_id;
        state.next_tab_id = id.wrapping_add(1).max(FIRST_TAB_ID);
        let position = state
            .tab_order
            .iter()
            .position(|open| *open == state.tab_id)
            .map_or(state.tab_order.len(), |index| index + 1);
        state.tab_order.insert(position, id);
        state.tab_id = id;
        new_tab = Some(id);
    }
    if let Err(error) = queue_path(window, path) {
        if let Some(id) = new_tab {
            let _ = close_tab(window, id);
        }
        return Err(error);
    }
    sync_tab_strip(window);
    Ok(())
}

fn open_documents(window: HWND, paths: Vec<PathBuf>) {
    let mut failures = 0_usize;
    let mut first_error = None;
    for path in paths {
        if let Err(error) = open_document(window, path) {
            failures += 1;
            first_error.get_or_insert(error);
        }
    }
    if let Some(error) = first_error {
        let message = if failures == 1 {
            format!("Could not open a file: {error}")
        } else {
            format!("Could not open {failures} files: {error}")
        };
        set_status(window, &message);
    }
}

/// Accepts files forwarded by a later launch. The paths are copied out and
/// opened from a posted message, after the sending process has been released.
fn receive_open_request(window: HWND, lparam: LPARAM) -> bool {
    let pointer = state_pointer_for(window);
    // SAFETY: WM_COPYDATA is dispatched on the UI thread that owns the state.
    let Some(state) = (unsafe { pointer.as_mut() }) else {
        return false;
    };
    // A closing window refuses, so the sender keeps trying and opens its own
    // window once this one has gone, instead of losing the files.
    if state.closing {
        return false;
    }
    let Some(paths) = process::read_open_request(lparam) else {
        return false;
    };
    let has_paths = !paths.is_empty();
    let room = process::MAX_FORWARDED_PATHS.saturating_sub(state.pending_opens.len());
    state.pending_opens.extend(paths.into_iter().take(room));
    // SAFETY: These calls act on this live top-level window.
    unsafe {
        if IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        }
        if IsWindowVisible(window).as_bool() {
            let _ = SetForegroundWindow(window);
        }
    }
    if has_paths {
        // SAFETY: This posts a pointer-free private message to the live window.
        let _ = unsafe { PostMessageW(Some(window), WM_APP_OPEN_PENDING, WPARAM(0), LPARAM(0)) };
    }
    true
}

fn open_pending(window: HWND) {
    let pointer = state_pointer_for(window);
    // SAFETY: Posted messages run on the UI thread that owns the state.
    let Some(paths) =
        (unsafe { pointer.as_mut() }).map(|state| std::mem::take(&mut state.pending_opens))
    else {
        return;
    };
    open_documents(window, paths);
}

/// Moves the active document out of the shared view, leaving the view empty.
fn stash_active(state: &mut WindowState) -> Result<BackgroundTab, ShellError> {
    let top_row = top_visible_row(state);
    let selected_row = first_selected_row(state);
    let column_widths = column_widths(state.list, state.columns);
    let chrome = capture_chrome(state);
    let rows = state.rows;
    let cache = Arc::clone(&state.cache);
    state.replace_cache(
        SlidingRowWindow::new(0, 0),
        Arc::new(ImmutableRowCache::default()),
    )?;
    Ok(BackgroundTab {
        id: state.tab_id,
        worker: state.worker.take(),
        path: state.current_path.take(),
        serial: state.active_serial.take(),
        columns: state.columns,
        column_widths,
        rows,
        cache,
        top_row,
        selected_row,
        pending_reveal: state.pending_reveal.take(),
        active_find: state.active_find.take(),
        revealed_match: state.revealed_match.take(),
        chrome,
    })
}

/// Shows a background tab's document in the shared view.
fn restore_tab(
    window: HWND,
    state: &mut WindowState,
    tab: BackgroundTab,
) -> Result<(), ShellError> {
    // The document moves in first, so a failed view update cannot lose it.
    state.tab_id = tab.id;
    state.worker = tab.worker;
    state.current_path = tab.path;
    state.active_serial = tab.serial;
    state.active_find = tab.active_find;
    state.revealed_match = tab.revealed_match;
    state.pending_reveal = None;

    reset_data_columns(state)?;
    insert_existing_data_columns(state.list, tab.columns)
        .map_err(|()| ShellError::new("native data-column restore failed"))?;
    state.columns = tab.columns;
    set_column_widths(state.list, &tab.column_widths);
    state.replace_cache(tab.rows, tab.cache)?;
    set_native_item_count(state.list, state.rows.visible_rows());
    apply_chrome(state, &tab.chrome)?;
    match state.current_path.as_deref() {
        Some(path) => set_document_title(window, path)?,
        None => initialize_window_title(window)?,
    }
    restore_view(state, tab.top_row, tab.selected_row);
    // SAFETY: The list is live and every visible row now belongs to this tab.
    let _ = unsafe { InvalidateRect(Some(state.list), None, true) };
    if let Some(row) = tab.pending_reveal {
        return begin_reveal(state, row);
    }
    if let (Some(worker), Some(serial), Some(top_row)) =
        (state.worker.as_ref(), state.active_serial, tab.top_row)
        && !state.cache.contains_row(top_row)
    {
        let _ = worker.request_viewport(serial, top_row);
    }
    Ok(())
}

fn activate_tab(window: HWND, id: usize) -> Result<(), ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: Commands and notifications run on the UI thread that owns the state.
    let state = unsafe { pointer.as_mut() }
        .ok_or_else(|| ShellError::new("window state is unavailable"))?;
    if id == state.tab_id {
        return Ok(());
    }
    let index = state
        .background_tabs
        .iter()
        .position(|tab| tab.id == id)
        .ok_or_else(|| ShellError::new("that tab is no longer open"))?;
    let outgoing = stash_active(state)?;
    let incoming = state.background_tabs.swap_remove(index);
    state.background_tabs.push(outgoing);
    let restored = restore_tab(window, state, incoming);
    sync_tab_strip(window);
    restored
}

/// Closes one tab. Closing the active tab shows its right neighbour, or its
/// left one at the end of the strip; closing the last tab shows the welcome
/// view in a fresh, empty tab.
fn close_tab(window: HWND, id: usize) -> Result<(), ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: Commands and posted messages run on the UI thread that owns the state.
    let state = unsafe { pointer.as_mut() }
        .ok_or_else(|| ShellError::new("window state is unavailable"))?;
    let Some(position) = state.tab_order.iter().position(|open| *open == id) else {
        return Ok(());
    };
    if id != state.tab_id {
        state.tab_order.remove(position);
        if let Some(index) = state.background_tabs.iter().position(|tab| tab.id == id) {
            // Dropping the tab stops its worker and closes its file.
            drop(state.background_tabs.swap_remove(index));
        }
        sync_tab_strip(window);
        return Ok(());
    }
    let neighbour = state
        .tab_order
        .get(position + 1)
        .or_else(|| {
            position
                .checked_sub(1)
                .and_then(|previous| state.tab_order.get(previous))
        })
        .copied();
    let closing = stash_active(state)?;
    state.tab_order.remove(position);
    drop(closing);
    let incoming = neighbour
        .and_then(|next| state.background_tabs.iter().position(|tab| tab.id == next))
        .map(|index| state.background_tabs.swap_remove(index));
    let shown = if let Some(incoming) = incoming {
        restore_tab(window, state, incoming)
    } else {
        let fresh = state.next_tab_id;
        state.next_tab_id = fresh.wrapping_add(1).max(FIRST_TAB_ID);
        state.tab_id = fresh;
        state.tab_order.push(fresh);
        show_welcome(window, state)
    };
    sync_tab_strip(window);
    shown
}

fn close_active_tab(window: HWND) -> Result<(), ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: Commands run on the UI thread that owns the state.
    let target = unsafe { pointer.as_ref() }
        .filter(|state| state.current_path.is_some() || state.tab_order.len() > 1)
        .map(|state| state.tab_id);
    match target {
        Some(id) => close_tab(window, id),
        None => Ok(()),
    }
}

fn switch_tab(window: HWND, forward: bool) {
    let pointer = state_pointer_for(window);
    // SAFETY: Commands run on the UI thread that owns the state.
    let target = unsafe { pointer.as_ref() }.and_then(|state| {
        let count = state.tab_order.len();
        let position = state.tab_order.iter().position(|id| *id == state.tab_id)?;
        if count < 2 {
            return None;
        }
        let next = if forward {
            (position + 1) % count
        } else {
            (position + count - 1) % count
        };
        state.tab_order.get(next).copied()
    });
    if let Some(id) = target
        && let Err(error) = activate_tab(window, id)
    {
        set_status(window, &format!("Could not switch tabs: {error}"));
    }
}

/// Ctrl+1 through Ctrl+8 pick a tab by position; Ctrl+9 picks the last one.
fn select_tab_number(window: HWND, command: u16) {
    let pointer = state_pointer_for(window);
    // SAFETY: Commands run on the UI thread that owns the state.
    let target = unsafe { pointer.as_ref() }.and_then(|state| {
        let index = if command == ID_TAB_SELECT_LAST {
            state.tab_order.len().checked_sub(1)?
        } else {
            usize::from(command.checked_sub(ID_TAB_SELECT_FIRST)?)
        };
        state.tab_order.get(index).copied()
    });
    if let Some(id) = target
        && let Err(error) = activate_tab(window, id)
    {
        set_status(window, &format!("Could not switch tabs: {error}"));
    }
}

fn handle_tab_notification(window: HWND, lparam: LPARAM) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    let pointer = state_pointer_for(window);
    // SAFETY: Every WM_NOTIFY payload starts with a readable NMHDR, and the
    // state stays attached for this synchronous callback.
    let (Some(state), header) = (unsafe { pointer.as_ref() }, unsafe {
        &*(lparam.0 as *const NMHDR)
    }) else {
        return false;
    };
    if header.hwndFrom != state.chrome.tab_strip {
        return false;
    }
    if header.code == TCN_SELCHANGE {
        let target = tabs::selected_index(state.chrome.tab_strip)
            .and_then(|index| state.tab_order.get(index).copied());
        if let Some(id) = target
            && let Err(error) = activate_tab(window, id)
        {
            set_status(window, &format!("Could not switch tabs: {error}"));
        }
    }
    true
}

fn tab_path(state: &WindowState, id: usize) -> Option<&Path> {
    if id == state.tab_id {
        return state.current_path.as_deref();
    }
    state
        .background_tabs
        .iter()
        .find(|tab| tab.id == id)
        .and_then(|tab| tab.path.as_deref())
}

/// Brings the tab strip in line with the open tabs. The strip is laid out
/// again only when its labels change, which covers opening and closing.
fn sync_tab_strip(window: HWND) {
    let pointer = state_pointer_for(window);
    // SAFETY: Callers run on the UI thread that owns the state.
    let Some(state) = (unsafe { pointer.as_mut() }) else {
        return;
    };
    let paths: Vec<Option<&Path>> = state
        .tab_order
        .iter()
        .map(|id| tab_path(state, *id))
        .collect();
    let labels = tabs::labels(&paths);
    let selected = state
        .tab_order
        .iter()
        .position(|id| *id == state.tab_id)
        .unwrap_or_default();
    if labels == state.tab_labels {
        tabs::select(state.chrome.tab_strip, selected);
        return;
    }
    tabs::set_items(state.chrome.tab_strip, &labels, selected);
    state.tab_labels = labels;
    state.tab_hover = TabHover::default();
    layout_children(window);
}

fn set_welcome_text(state: &WindowState) -> Result<(), ShellError> {
    set_control_text(state.chrome.file_name, IDLE_FILE_NAME)?;
    set_control_text(state.chrome.file_meta, IDLE_FILE_META)?;
    set_empty_state(state, WELCOME_EYEBROW, EMPTY_TITLE_CAPTION, WELCOME_BODY)?;
    set_control_text(state.status, IDLE_STATUS)
}

fn show_welcome(window: HWND, state: &mut WindowState) -> Result<(), ShellError> {
    reset_data_columns(state)?;
    set_native_item_count(state.list, 0);
    set_welcome_text(state)?;
    state.grid_visible = false;
    state.progress_visible = false;
    progress::update_position(state.chrome.progress, 0);
    apply_shell_visibility(state);
    initialize_window_title(window)
}

fn capture_chrome(state: &WindowState) -> TabChrome {
    // SAFETY: PBM_GETPOS is a pointer-free query of the live progress control.
    let progress = unsafe { SendMessageW(state.chrome.progress, PBM_GETPOS, None, None) }.0;
    TabChrome {
        file_name: control_text(state.chrome.file_name),
        file_meta: control_text(state.chrome.file_meta),
        empty_eyebrow: control_text(state.chrome.empty_eyebrow),
        empty_title: control_text(state.chrome.empty_title),
        empty_body: control_text(state.chrome.empty_body),
        status: control_text(state.status),
        grid_visible: state.grid_visible,
        progress_visible: state.progress_visible,
        progress_percent: u64::try_from(progress).unwrap_or_default(),
    }
}

fn apply_chrome(state: &mut WindowState, chrome: &TabChrome) -> Result<(), ShellError> {
    set_control_text(state.chrome.file_name, &chrome.file_name)?;
    set_control_text(state.chrome.file_meta, &chrome.file_meta)?;
    set_empty_state(
        state,
        &chrome.empty_eyebrow,
        &chrome.empty_title,
        &chrome.empty_body,
    )?;
    set_status_handle(state.status, &chrome.status);
    state.grid_visible = chrome.grid_visible;
    state.progress_visible = chrome.progress_visible;
    progress::update_position(state.chrome.progress, chrome.progress_percent);
    apply_shell_visibility(state);
    Ok(())
}

fn control_text(handle: HWND) -> String {
    // SAFETY: This is a read-only length query for a live child control.
    let length = unsafe { GetWindowTextLengthW(handle) };
    let length = usize::try_from(length)
        .unwrap_or_default()
        .min(MAX_CHROME_TEXT_UNITS);
    let mut buffer = vec![0_u16; length + 1];
    // SAFETY: The buffer has room for the text and its terminator.
    let copied = unsafe { GetWindowTextW(handle, &mut buffer) };
    let copied = usize::try_from(copied).unwrap_or_default().min(length);
    String::from_utf16_lossy(&buffer[..copied])
}

fn column_widths(list: HWND, columns: NativeColumns) -> Vec<i32> {
    (0..=usize::from(columns.data_columns))
        .map(|column| {
            // SAFETY: LVM_GETCOLUMNWIDTH carries a column index and no pointers.
            let width =
                unsafe { SendMessageW(list, LVM_GETCOLUMNWIDTH, Some(WPARAM(column)), None) }.0;
            i32::try_from(width).unwrap_or_default()
        })
        .collect()
}

fn set_column_widths(list: HWND, widths: &[i32]) {
    for (column, width) in widths.iter().enumerate() {
        if *width > 0 {
            // SAFETY: LVM_SETCOLUMNWIDTH carries a column index and a width.
            unsafe {
                SendMessageW(
                    list,
                    LVM_SETCOLUMNWIDTH,
                    Some(WPARAM(column)),
                    Some(LPARAM(isize::try_from(*width).unwrap_or_default())),
                );
            }
        }
    }
}

fn top_visible_row(state: &WindowState) -> Option<u64> {
    // SAFETY: LVM_GETTOPINDEX is a pointer-free query of the live list.
    let top = unsafe { SendMessageW(state.list, LVM_GETTOPINDEX, None, None) }.0;
    state.rows.local_to_absolute(i32::try_from(top).ok()?)
}

fn first_selected_row(state: &WindowState) -> Option<u64> {
    // SAFETY: Starting from -1 finds the first selected item; no pointers.
    let selected = unsafe {
        SendMessageW(
            state.list,
            LVM_GETNEXTITEM,
            Some(WPARAM(usize::MAX)),
            Some(LPARAM(isize::try_from(LVNI_SELECTED).unwrap_or_default())),
        )
    }
    .0;
    state.rows.local_to_absolute(i32::try_from(selected).ok()?)
}

fn restore_view(state: &WindowState, top_row: Option<u64>, selected_row: Option<u64>) {
    if let Some(local) = selected_row.and_then(|row| state.rows.absolute_to_local(row)) {
        let _ = mark_selected(state.list, local);
    }
    if let Some(local) = top_row.and_then(|row| state.rows.absolute_to_local(row)) {
        scroll_to_top(state.list, local, state.rows.visible_rows());
    }
}

/// Scrolls so `target` is the first visible row with one ensure-visible
/// request: the far edge of the page when moving down, the row itself when
/// moving up. Row indices never become pixel offsets, so large files are safe.
fn scroll_to_top(list: HWND, target: i32, count: u32) {
    // SAFETY: Both queries are pointer-free reads of the live list.
    let current = unsafe { SendMessageW(list, LVM_GETTOPINDEX, None, None) }.0;
    let Ok(current) = i32::try_from(current) else {
        return;
    };
    let final_row = i32::try_from(count).unwrap_or(i32::MAX).saturating_sub(1);
    if target == current || final_row < 0 {
        return;
    }
    let edge = if target > current {
        let per_page = unsafe { SendMessageW(list, LVM_GETCOUNTPERPAGE, None, None) }.0;
        let per_page = i32::try_from(per_page).unwrap_or(1).max(1);
        target.saturating_add(per_page - 1).min(final_row)
    } else {
        target
    };
    let Ok(edge) = usize::try_from(edge) else {
        return;
    };
    // SAFETY: The index is within the current item count; no pointers.
    unsafe {
        SendMessageW(list, LVM_ENSUREVISIBLE, Some(WPARAM(edge)), Some(LPARAM(0)));
    }
}

fn handle_drop(window: HWND, raw_drop: usize) {
    let paths = match drop_files::paths(raw_drop, MAX_OPEN_TABS) {
        Ok(paths) => paths,
        Err(error) => {
            set_status(window, &format!("Could not open dropped files: {error}"));
            return;
        }
    };
    let dropped = paths.len();
    let supported: Vec<PathBuf> = paths
        .into_iter()
        .filter(|path| is_supported_document_path(path))
        .collect();
    let skipped = dropped - supported.len();
    open_documents(window, supported);
    if skipped > 0 {
        set_status(
            window,
            &format!(
                "Skipped {skipped} unsupported file(s); use CSV, TSV, JSONL, NDJSON, LOG, or TXT"
            ),
        );
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

fn set_file_identity(
    state: &WindowState,
    path: &Path,
    source_bytes: Option<u64>,
) -> Result<(), ShellError> {
    let name = path
        .file_name()
        .filter(|value| !value.is_empty())
        .map_or_else(
            || path.as_os_str().to_string_lossy(),
            |value| value.to_string_lossy(),
        );
    set_control_text(state.chrome.file_name, &name)?;
    set_control_text(state.chrome.file_meta, &file_meta_text(source_bytes))
}

fn file_meta_text(source_bytes: Option<u64>) -> String {
    source_bytes.map_or_else(
        || String::from("Opening local file..."),
        |bytes| format!("{bytes} bytes | read-only source"),
    )
}

fn set_empty_state(
    state: &WindowState,
    eyebrow: &str,
    title: &str,
    body: &str,
) -> Result<(), ShellError> {
    set_control_text(state.chrome.empty_eyebrow, eyebrow)?;
    set_control_text(state.chrome.empty_title, title)?;
    set_control_text(state.chrome.empty_body, body)
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

fn apply_worker_event(window: HWND, tab_id: usize) {
    let pointer = state_pointer_for(window);
    // SAFETY: The UI thread owns this state and is the only event consumer.
    let Some(state) = (unsafe { pointer.as_mut() }) else {
        return;
    };
    if tab_id != state.tab_id {
        if let Some(tab) = state
            .background_tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            && let Some(event) = tab.worker.as_ref().and_then(Worker::poll)
        {
            tab.apply_event(&event);
        }
        return;
    }
    let Some(event) = state.worker.as_ref().and_then(Worker::poll) else {
        return;
    };
    if apply_document_event(window, &event) {
        // SAFETY: The document smoke timer, if present, belongs to this HWND.
        let _ = unsafe { KillTimer(Some(window), DOCUMENT_SMOKE_TIMER_ID) };
        let _ = destroy_shell_window(window);
    }
}

fn update_document_chrome(state: &mut WindowState, event: &WorkerEvent) {
    let chrome = event_chrome(event);
    let _ = set_file_identity(state, &event.path, chrome.source_bytes);
    state.progress_visible = chrome.progress_visible;
    progress::update_position(state.chrome.progress, chrome.progress_percent);
    state.grid_visible = chrome.grid_visible;
    if let Some([eyebrow, title, body]) = chrome.empty_state {
        let _ = set_empty_state(state, eyebrow, title, body);
    }
    apply_shell_visibility(state);
}

fn event_chrome(event: &WorkerEvent) -> EventChrome<'_> {
    let source_bytes = match &event.result {
        WorkerResult::Ready { size } => Some(*size),
        WorkerResult::Failed { .. } => None,
    };
    let (grid_visible, empty_state) = match &event.result {
        WorkerResult::Failed { message } => (
            false,
            Some([
                "COULD NOT OPEN",
                "This file could not be displayed.",
                message.as_str(),
            ]),
        ),
        WorkerResult::Ready { .. }
            if event.progress.complete && event.progress.available_rows == 0 =>
        {
            (
                false,
                Some([
                    "EMPTY FILE",
                    "This file has no rows.",
                    "Choose another supported local file to continue.",
                ]),
            )
        }
        WorkerResult::Ready { .. } if event.cached_rows > 0 => (true, None),
        WorkerResult::Ready { .. } => (
            false,
            Some([
                "INDEXING",
                "Finding the first complete row.",
                "Memory stays bounded while LeanRows scans forward cooperatively.",
            ]),
        ),
    };
    EventChrome {
        source_bytes,
        grid_visible,
        progress_visible: !matches!(event.phase, WorkerPhase::Complete | WorkerPhase::Failed),
        progress_percent: progress_percent(
            event.progress.scanned_bytes,
            event.progress.source_bytes,
        ),
        empty_state,
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

    update_document_chrome(state, event);

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
    if let Err(error) = redraw_shell_children(window) {
        set_status_handle(state.status, &format!("Display refresh failed: {error}"));
        return state.document_smoke;
    }

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
    let local_parameter = mark_selected(state.list, local_row)?;
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

/// Makes one row the only selected and focused row, without scrolling.
fn mark_selected(list: HWND, local_row: i32) -> Result<usize, ShellError> {
    let selection_state = LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0);
    let mut clear = LVITEMW {
        stateMask: selection_state,
        ..Default::default()
    };
    // SAFETY: -1 applies the supplied state mask to every virtual item; the
    // pointer remains valid through this synchronous call.
    let cleared = unsafe {
        SendMessageW(
            list,
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
            list,
            LVM_SETITEMSTATE,
            Some(WPARAM(local_parameter)),
            Some(LPARAM((&raw mut selected).cast::<c_void>() as isize)),
        )
    };
    if applied.0 == 0 {
        return Err(ShellError::new("target row could not be selected"));
    }
    Ok(local_parameter)
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
            " | File changed: press F5 or choose Reload; automatic reload is disabled"
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

fn destroy_shell_window(window: HWND) -> Result<(), ShellError> {
    let pointer = state_pointer_for(window);
    // SAFETY: The pointer slot belongs to this HWND and is valid until DestroyWindow returns.
    let cleanup = match unsafe { pointer.as_ref() } {
        Some(state) => state.clear_accessibility(),
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

    use super::layout::UiLayout;
    use super::{
        ActiveFind, EMPTY_TITLE_CAPTION, MAX_COPY_ROWS, MAX_COPY_UTF16_UNITS,
        MAX_STARTUP_ERROR_UTF16_UNITS, NativeColumns, RevealedMatch, SHELL_SMOKE_TEXT_UNITS,
        append_copy_units, build_copy_text, control_style, document_event_is_usable,
        document_smoke_evidence, document_title, event_chrome, failure_requires_reload,
        file_meta_text, find_needle, is_supported_document_path, parse_one_based_row,
        parse_open_selection, query_match_to_reveal, query_status, startup_error_text,
        worker_status,
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
    fn modern_shell_contract_is_bounded_and_actionable_at_minimum_size() {
        let layout = UiLayout::calculate(640, 480, 96, false);
        for rect in [
            layout.top_bar,
            layout.commands.search_field,
            layout.commands.open_button,
            layout.commands.overflow_button,
            layout.status_strip,
        ] {
            assert!(rect.width > 0 && rect.height > 0);
        }

        let title: Vec<u16> = EMPTY_TITLE_CAPTION.encode_utf16().collect();
        assert_eq!(
            String::from_utf16_lossy(&title),
            "Open large files without loading them all."
        );
        assert!(title.len() < SHELL_SMOKE_TEXT_UNITS);

        let button_type = control_style(windows::Win32::UI::WindowsAndMessaging::BS_OWNERDRAW).0
            & u32::try_from(windows::Win32::UI::WindowsAndMessaging::BS_TYPEMASK)
                .unwrap_or_default();
        assert_eq!(
            button_type,
            u32::try_from(windows::Win32::UI::WindowsAndMessaging::BS_OWNERDRAW)
                .unwrap_or_default()
        );
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
    fn open_dialog_selection_reads_one_or_several_files() {
        let one: Vec<u16> = "C:\\data\\rows.csv\0\0".encode_utf16().collect();
        assert_eq!(
            parse_open_selection(&one),
            vec![PathBuf::from(r"C:\data\rows.csv")]
        );
        let several: Vec<u16> = "C:\\data\0a.csv\0b.log\0\0".encode_utf16().collect();
        assert_eq!(
            parse_open_selection(&several),
            vec![
                PathBuf::from(r"C:\data\a.csv"),
                PathBuf::from(r"C:\data\b.log")
            ]
        );
        assert!(parse_open_selection(&[0, 0]).is_empty());
    }

    #[test]
    fn background_columns_grow_within_one_kind() -> Result<(), Box<dyn std::error::Error>> {
        let mut columns = NativeColumns {
            kind: None,
            data_columns: 0,
        };
        columns.grow(UiColumnLayout::fields(2).ok_or("invalid layout")?)?;
        columns.grow(UiColumnLayout::fields(1).ok_or("invalid layout")?)?;
        assert_eq!(
            columns,
            NativeColumns {
                kind: Some(UiColumnKind::Fields),
                data_columns: 2,
            }
        );
        assert!(columns.grow(UiColumnLayout::preview()).is_err());
        assert_eq!(columns.data_columns, 2);
        Ok(())
    }

    #[test]
    fn event_chrome_is_shared_by_active_and_background_tabs() {
        let ready = ready_event();
        let chrome = event_chrome(&ready);
        assert!(chrome.grid_visible && chrome.progress_visible);
        assert_eq!(chrome.empty_state, None);
        assert_eq!(chrome.source_bytes, Some(12));

        let mut failed = ready_event();
        failed.result = WorkerResult::Failed {
            message: String::from("access was denied"),
        };
        failed.phase = WorkerPhase::Failed;
        let chrome = event_chrome(&failed);
        assert!(!chrome.grid_visible && !chrome.progress_visible);
        assert_eq!(
            chrome.empty_state,
            Some([
                "COULD NOT OPEN",
                "This file could not be displayed.",
                "access was denied"
            ])
        );

        let mut empty = ready_event();
        empty.cached_rows = 0;
        empty.progress.complete = true;
        empty.progress.available_rows = 0;
        empty.phase = WorkerPhase::Complete;
        assert_eq!(
            event_chrome(&empty)
                .empty_state
                .map(|[eyebrow, _, _]| eyebrow),
            Some("EMPTY FILE")
        );
        assert_eq!(file_meta_text(None), "Opening local file...");
        assert_eq!(file_meta_text(Some(12)), "12 bytes | read-only source");
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
