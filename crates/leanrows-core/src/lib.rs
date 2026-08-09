//! Bounded-memory primitives for inspecting row-oriented files.
//!
//! The crate deliberately separates record discovery from record materialization.
//! Scanners emit byte spans while viewport reads copy only explicitly limited
//! prefixes. No API in this crate requires loading an entire file or record.

#![forbid(unsafe_code)]

mod budget;
mod cancel;
mod format;
mod index;
mod projection;
mod query;
mod scan;
mod session;
mod snapshot;
mod source;
mod types;
mod viewport;

pub use budget::{BudgetError, EngineLimits, ManagedMemoryBudget};
pub use cancel::{CancellationGeneration, CancellationToken};
pub use format::DocumentFormat;
pub use index::{
    AdaptiveByteIndex, AdaptiveRowIndex, CheckpointIndexError, CheckpointLookup, IndexUpdate,
};
pub use projection::{
    DelimitedRecordProjection, FieldProjection, FieldProjectionError, FieldProjectionLimits,
    FieldUtf8Status, project_delimited_record,
};
pub use query::{
    LiteralMatchMode, QUERY_HIT_ENTRY_BYTES, QUERY_SCRATCH_HEADER_BYTES, QueryError, QueryHit,
    QueryHitPage, QueryProgress, QueryQuotaExceeded, QueryRecoveryReport, QuerySession,
    QuerySessionConfig, QueryStepStatus, recover_stale_query_files,
};
pub use scan::{
    CsvBoundaryScanner, CsvDiagnostic, CsvDiagnosticKind, CsvDialect, CsvDialectError, CsvFinish,
    EmitControl, LineBoundaryScanner, LineFinish, ScanError, ScanProgress, ScanStatus,
};
pub use session::{
    ScanSession, ScanSessionConfig, ScanSessionError, ScanSessionProgress, ScanStep, ScanStepStatus,
};
pub use snapshot::{SourceChange, SourceFingerprint, SourceIdentity, SourceRevision};
pub use source::{FileSource, PositionedRead};
pub use types::{Checkpoint, RecordSpan, SpanError};
pub use viewport::{RecordPreview, Viewport, ViewportError, ViewportFormat, read_viewport};
