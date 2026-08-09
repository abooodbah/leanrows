use leanrows_core::{
    CancellationToken, CsvDiagnosticKind, CsvDialect, FileSource, LiteralMatchMode, PositionedRead,
    QUERY_HIT_ENTRY_BYTES, QUERY_SCRATCH_HEADER_BYTES, QueryError, QuerySession,
    QuerySessionConfig, QueryStepStatus, SourceChange, SourceFingerprint, SourceIdentity,
    SourceRevision, ViewportFormat, recover_stale_query_files,
};
use std::cell::{Cell, RefCell};
use std::error::Error;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const LARGE_QUOTA: u64 = QUERY_SCRATCH_HEADER_BYTES + 4_096 * QUERY_HIT_ENTRY_BYTES;
static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
struct MemorySource {
    bytes: Vec<u8>,
    largest_request: Cell<usize>,
    reads: Cell<u64>,
}

impl MemorySource {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            largest_request: Cell::new(0),
            reads: Cell::new(0),
        }
    }
}

impl PositionedRead for MemorySource {
    fn size(&self) -> io::Result<u64> {
        u64::try_from(self.bytes.len()).map_err(io::Error::other)
    }

    fn read_at(&self, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
        self.largest_request
            .set(self.largest_request.get().max(destination.len()));
        self.reads.set(self.reads.get().saturating_add(1));
        let start = usize::try_from(offset).map_err(io::Error::other)?;
        if start >= self.bytes.len() {
            return Ok(0);
        }
        let count = destination.len().min(self.bytes.len() - start);
        destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        Ok(count)
    }
}

#[derive(Clone, Copy, Debug)]
enum SourceMutation {
    Append,
    Truncate,
    Rewrite,
    Replace,
}

#[derive(Debug)]
struct MutableFingerprintSource {
    bytes: RefCell<Vec<u8>>,
    identity: Cell<u128>,
    revision: Cell<u128>,
    mutate_after_read: Cell<Option<SourceMutation>>,
}

impl MutableFingerprintSource {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes: RefCell::new(bytes),
            identity: Cell::new(1),
            revision: Cell::new(1),
            mutate_after_read: Cell::new(None),
        }
    }

    fn mutate(&self, mutation: SourceMutation) {
        match mutation {
            SourceMutation::Append => self.bytes.borrow_mut().push(b'!'),
            SourceMutation::Truncate => {
                self.bytes.borrow_mut().pop();
            }
            SourceMutation::Rewrite => {
                for byte in self.bytes.borrow_mut().iter_mut() {
                    *byte = b'Z';
                }
            }
            SourceMutation::Replace => {
                self.identity.set(self.identity.get().saturating_add(1));
                for byte in self.bytes.borrow_mut().iter_mut() {
                    *byte = b'R';
                }
            }
        }
        self.revision.set(self.revision.get().saturating_add(1));
    }

    fn mutate_on_next_read(&self, mutation: SourceMutation) {
        self.mutate_after_read.set(Some(mutation));
    }

    fn restore(&self, bytes: &[u8]) {
        self.bytes.replace(bytes.to_vec());
        self.identity.set(1);
        self.revision.set(1);
        self.mutate_after_read.set(None);
    }
}

impl PositionedRead for MutableFingerprintSource {
    fn size(&self) -> io::Result<u64> {
        u64::try_from(self.bytes.borrow().len()).map_err(io::Error::other)
    }

    fn fingerprint(&self) -> io::Result<SourceFingerprint> {
        Ok(SourceFingerprint::with_evidence(
            self.size()?,
            Some(SourceIdentity::new(9, self.identity.get())),
            Some(SourceRevision::new(self.revision.get(), 0)),
        ))
    }

    fn read_at(&self, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
        let start = usize::try_from(offset).map_err(io::Error::other)?;
        let bytes = self.bytes.borrow();
        if start >= bytes.len() {
            return Ok(0);
        }
        let count = destination.len().min(bytes.len() - start);
        destination[..count].copy_from_slice(&bytes[start..start + count]);
        drop(bytes);
        if let Some(mutation) = self.mutate_after_read.take() {
            self.mutate(mutation);
        }
        Ok(count)
    }
}

#[derive(Debug)]
struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> io::Result<Self> {
        let base = fs::canonicalize(std::env::temp_dir())?;
        for _ in 0..64 {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let name = format!("LeanRows-query-test-{}-{sequence}", std::process::id());
            let path = base.join(name);
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not reserve test directory",
        ))
    }

    fn child(&self, name: &str) -> io::Result<PathBuf> {
        let path = self.path.join(name);
        fs::create_dir(&path)?;
        Ok(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let Ok(base) = fs::canonicalize(std::env::temp_dir()) else {
            return;
        };
        if self.path.parent() == Some(base.as_path())
            && self
                .path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("LeanRows-query-test-"))
        {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn config(
    format: ViewportFormat,
    read_buffer: usize,
    page_capacity: usize,
    quota: u64,
) -> Result<QuerySessionConfig, QueryError> {
    QuerySessionConfig::new(format, read_buffer, 3, page_capacity, quota)
}

fn run_to_terminal<R: PositionedRead>(
    session: &mut QuerySession<R>,
    token: &CancellationToken,
    work_bytes: usize,
) -> Result<QueryStepStatus, QueryError> {
    let generation = token.snapshot();
    loop {
        match session.advance(work_bytes, token, generation)? {
            QueryStepStatus::Running => {}
            status => return Ok(status),
        }
    }
}

fn query_error<T>(
    result: Result<T, QueryError>,
    message: &'static str,
) -> Result<QueryError, Box<dyn Error>> {
    match result {
        Err(error) => Ok(error),
        Ok(_) => Err(message.into()),
    }
}

fn required_hit_quota(hit_count: u64) -> u64 {
    QUERY_SCRATCH_HEADER_BYTES + hit_count * QUERY_HIT_ENTRY_BYTES
}

#[test]
fn csv_quotes_and_embedded_newlines_use_logical_record_boundaries() -> Result<(), Box<dyn Error>> {
    let temporary = TestDirectory::new()?;
    let scratch = temporary.child("scratch")?;
    let input = b"row,value\n1,\"Needle\ninside\"\n2,end\n".to_vec();
    let source = MemorySource::new(input.clone());
    let mut session = QuerySession::new(
        source,
        b"needle\nINSIDE",
        LiteralMatchMode::AsciiCaseInsensitive,
        config(
            ViewportFormat::Delimited(CsvDialect::csv()),
            4,
            2,
            LARGE_QUOTA,
        )?,
        &scratch,
    )?;
    let token = CancellationToken::new();
    assert_eq!(
        run_to_terminal(&mut session, &token, 7)?,
        QueryStepStatus::Complete
    );
    assert_eq!(session.hit_count(), 1);
    let hit = session.read_hit(0)?.ok_or("missing CSV hit")?;
    assert_eq!(hit.row(), 1);
    let start = usize::try_from(hit.span().start())?;
    let end = usize::try_from(hit.span().end())?;
    assert_eq!(&input[start..end], b"Needle\ninside");

    let mut crossing = QuerySession::new(
        MemorySource::new(input),
        b"inside\"\n2",
        LiteralMatchMode::Exact,
        config(
            ViewportFormat::Delimited(CsvDialect::csv()),
            3,
            1,
            LARGE_QUOTA,
        )?,
        &scratch,
    )?;
    assert_eq!(
        run_to_terminal(&mut crossing, &token, 5)?,
        QueryStepStatus::Complete
    );
    assert_eq!(crossing.hit_count(), 0);
    Ok(())
}

#[test]
fn matcher_spans_chunks_but_not_line_delimiters() -> Result<(), Box<dyn Error>> {
    let temporary = TestDirectory::new()?;
    let scratch = temporary.child("scratch")?;
    let token = CancellationToken::new();
    let mut chunked = QuerySession::new(
        MemorySource::new(b"xxneedlezz\n".to_vec()),
        b"needle",
        LiteralMatchMode::Exact,
        config(ViewportFormat::Lines, 3, 1, LARGE_QUOTA)?,
        &scratch,
    )?;
    assert_eq!(
        run_to_terminal(&mut chunked, &token, 4)?,
        QueryStepStatus::Complete
    );
    let hit = chunked.read_hit(0)?.ok_or("missing chunk-boundary hit")?;
    assert_eq!((hit.span().start(), hit.span().end()), (2, 8));

    let mut crossing = QuerySession::new(
        MemorySource::new(b"ab\ncd\n".to_vec()),
        b"b\nc",
        LiteralMatchMode::Exact,
        config(ViewportFormat::Lines, 2, 1, LARGE_QUOTA)?,
        &scratch,
    )?;
    assert_eq!(
        run_to_terminal(&mut crossing, &token, 3)?,
        QueryStepStatus::Complete
    );
    assert_eq!(crossing.hit_count(), 0);
    Ok(())
}

#[test]
fn raw_invalid_utf8_and_ascii_folding_are_supported() -> Result<(), Box<dyn Error>> {
    let temporary = TestDirectory::new()?;
    let scratch = temporary.child("scratch")?;
    let input = vec![0xff, b'A', 0xfe, b'\n'];
    let mut session = QuerySession::new(
        MemorySource::new(input),
        &[0xff, b'a', 0xfe],
        LiteralMatchMode::AsciiCaseInsensitive,
        config(ViewportFormat::Lines, 2, 1, LARGE_QUOTA)?,
        &scratch,
    )?;
    let token = CancellationToken::new();
    assert_eq!(
        run_to_terminal(&mut session, &token, 2)?,
        QueryStepStatus::Complete
    );
    let hit = session.read_hit(0)?.ok_or("missing raw-byte hit")?;
    assert_eq!((hit.span().start(), hit.span().end()), (0, 3));
    Ok(())
}

#[test]
fn zero_length_query_is_rejected_before_scratch_creation() -> Result<(), Box<dyn Error>> {
    let temporary = TestDirectory::new()?;
    let scratch = temporary.child("scratch")?;
    let result = QuerySession::new(
        MemorySource::new(b"data\n".to_vec()),
        b"",
        LiteralMatchMode::Exact,
        config(ViewportFormat::Lines, 4, 1, LARGE_QUOTA)?,
        &scratch,
    );
    assert!(matches!(result, Err(QueryError::EmptyNeedle)));
    assert_eq!(fs::read_dir(scratch)?.count(), 0);
    Ok(())
}

#[test]
fn giant_record_keeps_matcher_reads_and_page_bounded() -> Result<(), Box<dyn Error>> {
    let temporary = TestDirectory::new()?;
    let scratch = temporary.child("scratch")?;
    let mut input = vec![b'x'; 512 * 1024];
    input.extend_from_slice(b"needle\n");
    let mut session = QuerySession::new(
        MemorySource::new(input),
        b"needle",
        LiteralMatchMode::Exact,
        config(ViewportFormat::Lines, 31, 1, LARGE_QUOTA)?,
        &scratch,
    )?;
    let token = CancellationToken::new();
    assert_eq!(
        run_to_terminal(&mut session, &token, 127)?,
        QueryStepStatus::Complete
    );
    assert_eq!(session.hit_count(), 1);
    assert!(session.source().largest_request.get() <= 31);
    assert!(session.source().reads.get() > 1);
    assert_eq!(session.read_page(0)?.hits().len(), 1);
    Ok(())
}

#[test]
fn malformed_eof_quote_is_diagnosed_without_hiding_hits() -> Result<(), Box<dyn Error>> {
    let temporary = TestDirectory::new()?;
    let scratch = temporary.child("scratch")?;
    let input = b"ok\n\"broken needle\nstill open".to_vec();
    let mut session = QuerySession::new(
        MemorySource::new(input),
        b"needle",
        LiteralMatchMode::Exact,
        config(
            ViewportFormat::Delimited(CsvDialect::csv()),
            5,
            1,
            LARGE_QUOTA,
        )?,
        &scratch,
    )?;
    let token = CancellationToken::new();
    assert_eq!(
        run_to_terminal(&mut session, &token, 6)?,
        QueryStepStatus::Complete
    );
    assert_eq!(session.hit_count(), 1);
    assert_eq!(
        session
            .read_hit(0)?
            .ok_or("missing malformed-row hit")?
            .row(),
        1
    );
    assert_eq!(
        session
            .progress()
            .csv_diagnostic()
            .ok_or("missing EOF diagnostic")?
            .kind(),
        CsvDiagnosticKind::UnterminatedQuotedField
    );
    Ok(())
}

#[test]
fn all_rows_match_at_quota_minus_exact_and_plus() -> Result<(), Box<dyn Error>> {
    let temporary = TestDirectory::new()?;
    let scratch = temporary.child("scratch")?;
    let token = CancellationToken::new();
    let exact_quota = required_hit_quota(3);

    let mut below = QuerySession::new(
        MemorySource::new(b"x\nx\nx\n".to_vec()),
        b"x",
        LiteralMatchMode::Exact,
        config(ViewportFormat::Lines, 2, 2, exact_quota - 1)?,
        &scratch,
    )?;
    let below_status = run_to_terminal(&mut below, &token, 3)?;
    let QueryStepStatus::QuotaExceeded(details) = below_status else {
        return Err("quota-minus session should be exhausted".into());
    };
    assert_eq!(details.quota_bytes(), exact_quota - 1);
    assert_eq!(details.used_bytes(), required_hit_quota(2));
    assert_eq!(details.required_bytes(), exact_quota);
    assert_eq!(details.stored_hits(), 2);
    assert_eq!(details.rejected_hit().row(), 2);
    assert_eq!(below.source().size()?, 6);
    assert_eq!(below.advance(3, &token, token.snapshot())?, below_status);

    for quota in [exact_quota, exact_quota + 1] {
        let mut fitting = QuerySession::new(
            MemorySource::new(b"x\nx\nx\n".to_vec()),
            b"x",
            LiteralMatchMode::Exact,
            config(ViewportFormat::Lines, 2, 2, quota)?,
            &scratch,
        )?;
        assert_eq!(
            run_to_terminal(&mut fitting, &token, 3)?,
            QueryStepStatus::Complete
        );
        assert_eq!(fitting.hit_count(), 3);
        assert_eq!(
            fs::metadata(fitting.scratch_path())?.len(),
            required_hit_quota(3)
        );
    }
    Ok(())
}

#[test]
fn cancellation_is_resumable_and_stale_generations_do_no_more_work() -> Result<(), Box<dyn Error>> {
    let temporary = TestDirectory::new()?;
    let scratch = temporary.child("scratch")?;
    let mut session = QuerySession::new(
        MemorySource::new(b"abcdef\n".to_vec()),
        b"cd",
        LiteralMatchMode::Exact,
        config(ViewportFormat::Lines, 2, 1, LARGE_QUOTA)?,
        &scratch,
    )?;
    let token = CancellationToken::new();
    let stale = token.snapshot();
    assert_eq!(session.advance(2, &token, stale)?, QueryStepStatus::Running);
    let before = session.progress();
    let reads_before = session.source().reads.get();
    token.cancel();
    assert_eq!(
        session.advance(16, &token, stale)?,
        QueryStepStatus::Cancelled
    );
    assert_eq!(session.progress(), before);
    assert_eq!(session.source().reads.get(), reads_before);
    assert_eq!(
        run_to_terminal(&mut session, &token, 2)?,
        QueryStepStatus::Complete
    );
    assert_eq!(session.hit_count(), 1);
    Ok(())
}

#[test]
fn query_source_changes_are_detected_before_more_state_is_consumed_and_stay_terminal()
-> Result<(), Box<dyn Error>> {
    let temporary = TestDirectory::new()?;
    let scratch = temporary.child("scratch")?;
    let original = b"alpha\nbeta\n".to_vec();
    let cases = [
        (SourceMutation::Append, SourceChange::Grew),
        (SourceMutation::Truncate, SourceChange::Shrank),
        (SourceMutation::Rewrite, SourceChange::RevisionChanged),
        (SourceMutation::Replace, SourceChange::IdentityChanged),
    ];

    for (mutation, expected_change) in cases {
        let source = MutableFingerprintSource::new(original.clone());
        let mut session = QuerySession::new(
            source,
            b"alpha",
            LiteralMatchMode::Exact,
            config(ViewportFormat::Lines, 2, 1, LARGE_QUOTA)?,
            &scratch,
        )?;
        let token = CancellationToken::new();
        let generation = token.snapshot();
        assert_eq!(
            session.advance(2, &token, generation)?,
            QueryStepStatus::Running
        );
        let before = session.progress();
        session.source().mutate(mutation);
        let error = query_error(
            session.advance(2, &token, generation),
            "mutated source must fail closed",
        )?;
        assert!(error.is_source_integrity_failure());
        match (expected_change, error) {
            (SourceChange::Grew, QueryError::SourceGrew { .. })
            | (SourceChange::Shrank, QueryError::SourceShrank) => {}
            (
                expected @ (SourceChange::RevisionChanged | SourceChange::IdentityChanged),
                QueryError::SourceChanged { change, .. },
            ) => assert_eq!(change, expected),
            (_, other) => return Err(format!("unexpected query mutation error: {other}").into()),
        }
        assert_eq!(session.progress(), before);

        // Restoring all externally observable evidence cannot unpoison a
        // session that may already have observed a conflicting generation.
        session.source().restore(&original);
        let repeated = query_error(
            session.advance(2, &token, generation),
            "source failure must remain terminal",
        )?;
        assert!(repeated.is_source_integrity_failure());
        assert_eq!(session.progress(), before);
    }
    Ok(())
}

#[test]
fn query_mid_read_rewrite_is_discarded_before_scanner_or_matcher_state_changes()
-> Result<(), Box<dyn Error>> {
    let temporary = TestDirectory::new()?;
    let scratch = temporary.child("scratch")?;
    let original = b"needle\nother\n".to_vec();
    let source = MutableFingerprintSource::new(original.clone());
    source.mutate_on_next_read(SourceMutation::Rewrite);
    let mut session = QuerySession::new(
        source,
        b"needle",
        LiteralMatchMode::Exact,
        config(ViewportFormat::Lines, 8, 1, LARGE_QUOTA)?,
        &scratch,
    )?;
    let token = CancellationToken::new();
    let generation = token.snapshot();
    assert!(matches!(
        session.advance(8, &token, generation),
        Err(QueryError::SourceChanged {
            change: SourceChange::RevisionChanged,
            ..
        })
    ));
    assert_eq!(session.progress().scanned_bytes(), 0);
    assert_eq!(session.progress().discovered_records(), 0);
    assert_eq!(session.hit_count(), 0);

    session.source().restore(&original);
    assert!(
        query_error(
            session.advance(8, &token, generation),
            "mid-read mismatch must remain terminal",
        )?
        .is_source_integrity_failure()
    );
    assert_eq!(session.progress().scanned_bytes(), 0);
    Ok(())
}

#[test]
fn stale_query_generation_yields_before_observing_a_changed_source() -> Result<(), Box<dyn Error>> {
    let temporary = TestDirectory::new()?;
    let scratch = temporary.child("scratch")?;
    let source = MutableFingerprintSource::new(b"alpha\nbeta\n".to_vec());
    let mut session = QuerySession::new(
        source,
        b"alpha",
        LiteralMatchMode::Exact,
        config(ViewportFormat::Lines, 4, 1, LARGE_QUOTA)?,
        &scratch,
    )?;
    let token = CancellationToken::new();
    let stale = token.snapshot();
    token.cancel();
    session.source().mutate(SourceMutation::Rewrite);
    let before = session.progress();
    assert_eq!(
        session.advance(4, &token, stale)?,
        QueryStepStatus::Cancelled
    );
    assert_eq!(session.progress(), before);
    assert!(matches!(
        session.advance(4, &token, token.snapshot()),
        Err(QueryError::SourceChanged {
            change: SourceChange::RevisionChanged,
            ..
        })
    ));
    assert_eq!(session.progress(), before);
    Ok(())
}

#[test]
fn fixed_page_supports_navigation_and_arbitrary_reads() -> Result<(), Box<dyn Error>> {
    let temporary = TestDirectory::new()?;
    let scratch = temporary.child("scratch")?;
    let mut session = QuerySession::new(
        MemorySource::new(b"x x x x\n".to_vec()),
        b"x",
        LiteralMatchMode::Exact,
        config(ViewportFormat::Lines, 3, 2, LARGE_QUOTA)?,
        &scratch,
    )?;
    let token = CancellationToken::new();
    assert_eq!(
        run_to_terminal(&mut session, &token, 5)?,
        QueryStepStatus::Complete
    );
    assert_eq!(session.hit_count(), 4);
    let first_page = session.read_page(0)?;
    assert_eq!(first_page.first_hit(), 0);
    assert_eq!(first_page.hits().len(), 2);
    assert_eq!(first_page.hits()[0].span().start(), 0);
    assert_eq!(first_page.hits()[1].span().start(), 2);
    assert_eq!(
        session
            .read_hit(3)?
            .ok_or("missing fourth hit")?
            .span()
            .start(),
        6
    );
    assert_eq!(session.next_hit_index(None), Some(0));
    assert_eq!(session.next_hit_index(Some(2)), Some(3));
    assert_eq!(session.next_hit_index(Some(3)), None);
    assert_eq!(session.previous_hit_index(None), Some(3));
    assert_eq!(session.previous_hit_index(Some(3)), Some(2));
    assert_eq!(session.previous_hit_index(Some(0)), None);
    Ok(())
}

#[test]
fn scratch_is_removed_on_drop_and_valid_stale_copies_are_recovered() -> Result<(), Box<dyn Error>> {
    let temporary = TestDirectory::new()?;
    let active = temporary.child("active")?;
    let recovery = temporary.child("recovery")?;
    let owned_path;
    let stale_path;
    {
        let mut session = QuerySession::new(
            MemorySource::new(b"hit\n".to_vec()),
            b"hit",
            LiteralMatchMode::Exact,
            config(ViewportFormat::Lines, 4, 1, LARGE_QUOTA)?,
            &active,
        )?;
        let token = CancellationToken::new();
        assert_eq!(
            run_to_terminal(&mut session, &token, 4)?,
            QueryStepStatus::Complete
        );
        owned_path = session.scratch_path().to_path_buf();
        let name = owned_path
            .file_name()
            .ok_or("scratch path has no file name")?;
        stale_path = recovery.join(name);
        fs::copy(&owned_path, &stale_path)?;
        assert!(owned_path.exists());
    }
    assert!(!owned_path.exists());
    assert!(stale_path.exists());

    let unrelated = recovery.join("notes.txt");
    fs::write(&unrelated, b"keep")?;
    let invalid = recovery.join("leanrows-query-v1-00000000000000000000000000000000.hits");
    fs::write(&invalid, b"partial")?;
    let candidate_directory =
        recovery.join("leanrows-query-v1-11111111111111111111111111111111.hits");
    fs::create_dir(&candidate_directory)?;
    let report = recover_stale_query_files(&recovery)?;
    assert_eq!(report.removed_files(), 1);
    assert_eq!(report.retained_candidates(), 2);
    assert!(!stale_path.exists());
    assert!(unrelated.exists());
    assert!(invalid.exists());
    assert!(candidate_directory.is_dir());
    Ok(())
}

#[test]
fn scratch_path_validation_rejects_files_and_never_writes_by_source() -> Result<(), Box<dyn Error>>
{
    let temporary = TestDirectory::new()?;
    let source_directory = temporary.child("source")?;
    let scratch_directory = temporary.child("scratch")?;
    let source_path = source_directory.join("rows.log");
    let mut source_file = File::create(&source_path)?;
    source_file.write_all(b"needle\n")?;
    drop(source_file);

    let mut session = QuerySession::new(
        FileSource::open(&source_path)?,
        b"needle",
        LiteralMatchMode::Exact,
        config(ViewportFormat::Lines, 4, 1, LARGE_QUOTA)?,
        &scratch_directory,
    )?;
    assert_eq!(fs::read_dir(&source_directory)?.count(), 1);
    assert_eq!(fs::read_dir(&scratch_directory)?.count(), 1);
    let token = CancellationToken::new();
    assert_eq!(
        run_to_terminal(&mut session, &token, 4)?,
        QueryStepStatus::Complete
    );
    assert_eq!(fs::read_dir(&source_directory)?.count(), 1);

    let not_a_directory = temporary.path.join("not-a-directory");
    fs::write(&not_a_directory, b"file")?;
    let rejected = QuerySession::new(
        MemorySource::new(b"data\n".to_vec()),
        b"data",
        LiteralMatchMode::Exact,
        config(ViewportFormat::Lines, 4, 1, LARGE_QUOTA)?,
        &not_a_directory,
    );
    assert!(matches!(rejected, Err(QueryError::UnsafeScratchDirectory)));
    assert!(matches!(
        recover_stale_query_files(&not_a_directory),
        Err(QueryError::UnsafeScratchDirectory)
    ));
    Ok(())
}

#[cfg(unix)]
#[test]
fn recovery_retains_symlink_candidates() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::symlink;

    let temporary = TestDirectory::new()?;
    let active = temporary.child("active")?;
    let recovery = temporary.child("recovery")?;
    let session = QuerySession::new(
        MemorySource::new(b"data\n".to_vec()),
        b"data",
        LiteralMatchMode::Exact,
        config(ViewportFormat::Lines, 4, 1, LARGE_QUOTA)?,
        &active,
    )?;
    let name = session
        .scratch_path()
        .file_name()
        .ok_or("scratch path has no file name")?;
    let link = recovery.join(name);
    symlink(session.scratch_path(), &link)?;
    let report = recover_stale_query_files(&recovery)?;
    assert_eq!(report.removed_files(), 0);
    assert_eq!(report.retained_candidates(), 1);
    assert!(fs::symlink_metadata(link)?.file_type().is_symlink());
    Ok(())
}

#[test]
fn invalid_resource_limits_are_structured() -> Result<(), Box<dyn Error>> {
    assert!(matches!(
        QuerySessionConfig::new(ViewportFormat::Lines, 0, 1, 1, LARGE_QUOTA),
        Err(QueryError::ZeroReadBuffer)
    ));
    assert!(matches!(
        QuerySessionConfig::new(ViewportFormat::Lines, 1, 0, 1, LARGE_QUOTA),
        Err(QueryError::ZeroCancellationInterval)
    ));
    assert!(matches!(
        QuerySessionConfig::new(ViewportFormat::Lines, 1, 1, 0, LARGE_QUOTA),
        Err(QueryError::ZeroPageCapacity)
    ));
    assert!(matches!(
        QuerySessionConfig::new(
            ViewportFormat::Lines,
            1,
            1,
            1,
            QUERY_SCRATCH_HEADER_BYTES - 1,
        ),
        Err(QueryError::ScratchQuotaTooSmall { .. })
    ));

    let temporary = TestDirectory::new()?;
    let scratch = temporary.child("scratch")?;
    let mut session = QuerySession::new(
        MemorySource::new(b"data\n".to_vec()),
        b"data",
        LiteralMatchMode::Exact,
        config(ViewportFormat::Lines, 1, 1, LARGE_QUOTA)?,
        &scratch,
    )?;
    let token = CancellationToken::new();
    assert!(matches!(
        session.advance(0, &token, token.snapshot()),
        Err(QueryError::ZeroWorkBudget)
    ));
    Ok(())
}
