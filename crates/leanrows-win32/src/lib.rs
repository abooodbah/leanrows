#![deny(unsafe_code)]

//! Native Windows shell primitives for `LeanRows`.
//!
//! The public API is safe Rust. All Win32 pointer and handle operations live in
//! the private `native` module, whose module-level lint exception is deliberate
//! and narrow.

mod accessibility;
mod document_engine;
mod mailbox;
mod model;
#[cfg(windows)]
#[allow(unsafe_code)]
mod native;
#[cfg(not(windows))]
mod native_stub;
mod worker;

use std::fmt;
use std::path::PathBuf;

pub use accessibility::{AccessibleName, MAX_ACCESSIBLE_NAME_UTF16_UNITS};
pub use mailbox::{CoalescingMailbox, MailboxClosed, SendOutcome};
pub use model::{CachedRow, DisplayCell, ImmutableRowCache, SlidingRowWindow};

/// Options used to start the native shell.
#[derive(Debug, Default)]
pub struct ShellOptions {
    /// Optional file supplied on the command line.
    pub initial_path: Option<PathBuf>,
    /// Create and verify the native controls, then exit without user input.
    pub smoke_test: bool,
    /// Verify that one real document row reaches the owner-data grid, then exit.
    pub document_smoke_test: bool,
}

/// Worker and grid evidence captured by the document smoke path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentSmokeEvidence {
    /// Opened source size reported by the worker.
    pub source_bytes: u64,
    /// Absolute row at the beginning of the transferred immutable cache.
    pub cache_first_row: u64,
    /// Number of rows retained in the transferred cache.
    pub cached_rows: u32,
    /// Bounded row-number cell actually retained for the first row.
    pub first_row: String,
    /// Bounded preview cell actually retained for the first row.
    pub preview: String,
    /// Semantic shape of the data columns displayed after the row number.
    pub column_kind: DocumentSmokeColumnKind,
    /// Bounded number of data columns exposed by the worker event.
    pub data_columns: u8,
    /// Up to four independently decoded first-row data cells.
    pub sample_cells: Vec<String>,
    /// Bytes scanned when the first usable worker event reached the grid.
    pub scanned_bytes: u64,
    /// Rows indexed when the first usable worker event reached the grid.
    pub indexed_rows: u64,
    /// Rows made available to the native virtual list at that point.
    pub available_rows: u64,
    /// Whether full indexing had already completed.
    pub scan_complete: bool,
    /// Worker phase that supplied the cache.
    pub phase: DocumentSmokePhase,
}

/// Column shape observed by the real document smoke path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentSmokeColumnKind {
    /// LOG, TXT, JSONL, and NDJSON expose one preview column.
    Preview,
    /// CSV and TSV expose independently decoded field columns.
    Fields,
}

/// Successful worker phase observed by the document smoke path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentSmokePhase {
    /// The first bounded viewport was ready before full indexing.
    ViewportReady,
    /// A later incremental progress event supplied the current cache.
    Scanning,
    /// The source was fully indexed before the event was consumed.
    Complete,
}

/// Evidence returned after the shell message loop exits.
#[allow(
    clippy::struct_excessive_bools,
    reason = "each smoke flag records an independent fail-closed native assertion"
)]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShellOutcome {
    /// True only when smoke mode verified both native child controls.
    pub smoke_controls_verified: bool,
    /// True only when smoke mode retrieved the expected owner-data item name.
    pub smoke_accessibility_verified: bool,
    /// True only when smoke mode transferred keyboard focus to the row grid.
    pub smoke_keyboard_focus_verified: bool,
    /// True only when smoke mode verified the system-color/high-contrast path.
    pub smoke_system_colors_verified: bool,
    /// Real document evidence, present only after a successful document smoke.
    pub document_smoke: Option<DocumentSmokeEvidence>,
}

/// A shell startup or message-loop failure.
#[derive(Debug)]
pub struct ShellError {
    message: String,
}

impl ShellError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ShellError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ShellError {}

/// Runs the platform shell until its main window closes.
///
/// # Errors
///
/// Returns [`ShellError`] if native initialization, control creation, or the
/// Windows message loop fails.
pub fn run_shell(options: ShellOptions) -> Result<ShellOutcome, ShellError> {
    #[cfg(windows)]
    {
        native::run(options)
    }

    #[cfg(not(windows))]
    {
        native_stub::run(options)
    }
}

/// Shows a bounded native error dialog for an ordinary interactive startup.
///
/// Automation callers should write diagnostics to their redirected standard
/// handles instead; this function is intentionally modal.
pub fn show_startup_error(message: &str) {
    #[cfg(windows)]
    {
        native::show_startup_error(message);
    }

    #[cfg(not(windows))]
    {
        native_stub::show_startup_error(message);
    }
}
