//! External single-column sorting with fixed-memory, cancellable work steps.
#![allow(clippy::missing_errors_doc)]
mod key;
mod session;
mod storage;
use crate::ViewportFormat;
pub use session::SortSession;
use std::{fmt, io};
pub use storage::{
    SortMemoryReservation, SortResources, SortedOrder, SortedRecord, recover_stale_sort_files,
};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortMode {
    Text,
    Numeric,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortDirection {
    Ascending,
    Descending,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SortRequest {
    pub column: usize,
    pub mode: SortMode,
    pub direction: SortDirection,
}
#[derive(Clone, Copy, Debug)]
pub struct SortConfig {
    pub format: ViewportFormat,
    pub request: SortRequest,
    pub batch_capacity: usize,
    pub merge_fan_in: usize,
    pub step_bytes: usize,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortPhase {
    Extracting,
    SortingRuns,
    Merging,
    BuildingInverse,
    Complete,
    Cancelled,
}
#[derive(Clone, Copy, Debug)]
pub struct SortStep {
    pub phase: SortPhase,
    pub scanned_bytes: u64,
    pub source_bytes: u64,
    pub records: u64,
}
#[derive(Debug)]
pub enum SortError {
    Io(io::Error),
    SourceChanged,
    Allocation,
    Quota,
    Cancelled,
    NumericRange,
    InvalidScratch,
    InvalidConfig,
    Scan(crate::ScanError),
}
impl fmt::Display for SortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "sort I/O failed: {e}"),
            Self::SourceChanged => f.write_str("sort source changed; reload required"),
            Self::Allocation => f.write_str("sort memory reservation unavailable"),
            Self::Quota => f.write_str("sort scratch quota exceeded"),
            Self::Cancelled => f.write_str("sort cancelled"),
            Self::NumericRange => f.write_str("numeric exponent exceeds supported range"),
            Self::InvalidScratch => f.write_str("sort scratch data is invalid"),
            Self::InvalidConfig => f.write_str("sort resource configuration is invalid"),
            Self::Scan(e) => write!(f, "sort record scan failed: {e}"),
        }
    }
}
impl std::error::Error for SortError {}
impl From<io::Error> for SortError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<crate::ScanError> for SortError {
    fn from(e: crate::ScanError) -> Self {
        Self::Scan(e)
    }
}
