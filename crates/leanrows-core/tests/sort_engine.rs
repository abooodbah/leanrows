use leanrows_core::{
    CancellationToken, CsvDialect, PositionedRead, SortConfig, SortDirection, SortMode, SortPhase,
    SortRequest, SortResources, SortSession, ViewportFormat,
};
use std::fmt::Write as _;
use std::{
    cell::Cell,
    error::Error,
    fs, io,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct ChangedSource {
    bytes: Fixture,
    revision: std::rc::Rc<Cell<u128>>,
    change_on_read: bool,
}
impl PositionedRead for ChangedSource {
    fn size(&self) -> io::Result<u64> {
        self.bytes.size()
    }
    fn fingerprint(&self) -> io::Result<leanrows_core::SourceFingerprint> {
        Ok(leanrows_core::SourceFingerprint::with_evidence(
            self.size()?,
            None,
            Some(leanrows_core::SourceRevision::new(self.revision.get(), 0)),
        ))
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> io::Result<usize> {
        let count = self.bytes.read_at(offset, out)?;
        if self.change_on_read {
            self.revision.set(self.revision.get() + 1);
        }
        Ok(count)
    }
}

#[test]
fn same_size_source_change_never_publishes_an_order() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let resources = SortResources::new(16 * 1024 * 1024, 1024 * 1024);
    let mut session = SortSession::new(
        ChangedSource {
            bytes: Fixture(b"b\na\n".to_vec()),
            revision: std::rc::Rc::new(Cell::new(0)),
            change_on_read: true,
        },
        config(SortMode::Text, SortDirection::Ascending, 0),
        resources.clone(),
        &scratch.0,
    )?;
    let token = CancellationToken::new();
    assert!(matches!(
        session.advance(&token, token.snapshot()),
        Err(leanrows_core::SortError::SourceChanged)
    ));
    drop(session);
    assert_eq!(resources.disk_bytes(), 0);
    assert_eq!(resources.memory_bytes(), 0);
    Ok(())
}
static NEXT: AtomicU64 = AtomicU64::new(0);
#[test]
fn source_change_after_completion_rejects_publication() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let resources = SortResources::new(16 * 1024 * 1024, 1024 * 1024);
    let revision = std::rc::Rc::new(Cell::new(0));
    let mut session = SortSession::new(
        ChangedSource {
            bytes: Fixture(b"b\na\n".to_vec()),
            revision: revision.clone(),
            change_on_read: false,
        },
        config(SortMode::Text, SortDirection::Ascending, 0),
        resources.clone(),
        &scratch.0,
    )?;
    let token = CancellationToken::new();
    while session.advance(&token, token.snapshot())?.phase != SortPhase::Complete {}
    revision.set(1);
    assert!(matches!(
        session.into_order(),
        Err(leanrows_core::SortError::SourceChanged)
    ));
    assert_eq!(resources.disk_bytes(), 0);
    assert_eq!(resources.memory_bytes(), 0);
    Ok(())
}
struct Fixture(Vec<u8>);
#[cfg(windows)]
#[test]
fn blocked_cleanup_keeps_disk_quota_until_retry() -> Result<(), Box<dyn Error>> {
    use std::os::windows::fs::OpenOptionsExt;
    let scratch = Scratch::new()?;
    let resources = SortResources::new(16 * 1024 * 1024, 1024 * 1024);
    let mut session = SortSession::new(
        Fixture(b"b\na\n".to_vec()),
        config(SortMode::Text, SortDirection::Ascending, 0),
        resources.clone(),
        &scratch.0,
    )?;
    let token = CancellationToken::new();
    while session.advance(&token, token.snapshot())?.phase != SortPhase::Complete {}
    let order = session.into_order()?;
    let directory = fs::read_dir(&scratch.0)?
        .next()
        .ok_or("missing owner")??
        .path();
    let blocked = fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(directory.join("inverse.bin"))?;
    drop(order);
    assert!(
        resources.disk_bytes() > 0,
        "retained file lost its disk reservation"
    );
    drop(blocked);
    resources.retry_cleanup();
    assert_eq!(resources.disk_bytes(), 0);
    assert_eq!(resources.memory_bytes(), 0);
    assert!(!directory.exists());
    Ok(())
}
impl PositionedRead for Fixture {
    fn size(&self) -> io::Result<u64> {
        Ok(self.0.len() as u64)
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> io::Result<usize> {
        let start = usize::try_from(offset).map_err(io::Error::other)?;
        let bytes = self.0.get(start..).unwrap_or_default();
        let n = bytes.len().min(out.len());
        out[..n].copy_from_slice(&bytes[..n]);
        Ok(n)
    }
}
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "leanrows-sort-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn config(mode: SortMode, direction: SortDirection, column: usize) -> SortConfig {
    SortConfig {
        format: ViewportFormat::Delimited(CsvDialect::csv()),
        request: SortRequest {
            column,
            mode,
            direction,
        },
        batch_capacity: 2,
        merge_fan_in: 2,
        step_bytes: 17,
    }
}
fn rows(input: &[u8], config: SortConfig) -> Result<Vec<u64>, Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let resources = SortResources::new(16 * 1024 * 1024, 8 * 1024 * 1024 * 1024);
    let mut session = SortSession::new(
        Fixture(input.to_vec()),
        config,
        resources.clone(),
        &scratch.0,
    )?;
    let token = CancellationToken::new();
    while session.advance(&token, token.snapshot())?.phase != SortPhase::Complete {}
    let order = session.into_order()?;
    let mut result = Vec::new();
    for rank in 0..order.len() {
        let row = order.record_at(rank)?.source_row;
        assert_eq!(order.rank_of(row)?, rank);
        result.push(row);
    }
    assert!(resources.memory_peak() <= 16 * 1024 * 1024);
    drop(order);
    assert_eq!(resources.disk_bytes(), 0);
    Ok(result)
}
#[test]
fn tiny_batches_force_global_merge() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        rows(
            b"z\nc\na\ny\nb\nx\n",
            config(SortMode::Text, SortDirection::Ascending, 0)
        )?,
        vec![2, 4, 1, 5, 3, 0]
    );
    Ok(())
}
#[test]
fn exact_numeric_order_and_stable_ties() -> Result<(), Box<dyn Error>> {
    let input = b"9007199254740993\n9007199254740992\n-3\n2.00\n2e0\n\nx\n-0\n0\n";
    assert_eq!(
        rows(
            input,
            config(SortMode::Numeric, SortDirection::Ascending, 0)
        )?,
        vec![2, 7, 8, 3, 4, 1, 0, 6, 5]
    );
    assert_eq!(
        rows(
            input,
            config(SortMode::Numeric, SortDirection::Descending, 0)
        )?,
        vec![0, 1, 3, 4, 7, 8, 2, 6, 5]
    );
    Ok(())
}
#[test]
fn full_multiline_escaped_fields_and_missing_columns() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        rows(
            b"1,\"z\nz\"\r\n2,\"a\"\"b\"\r\n3,\r\n4\r\n",
            config(SortMode::Text, SortDirection::Ascending, 1)
        )?,
        vec![2, 3, 1, 0]
    );
    let mut input = vec![b'a'; 5000];
    input.extend_from_slice(b"z\n");
    input.extend(std::iter::repeat_n(b'a', 5000));
    input.extend_from_slice(b"b\n");
    assert_eq!(
        rows(&input, config(SortMode::Text, SortDirection::Ascending, 0))?,
        vec![1, 0]
    );
    Ok(())
}
#[test]
fn empty_single_and_invalid_bytes() -> Result<(), Box<dyn Error>> {
    assert!(rows(b"", config(SortMode::Text, SortDirection::Ascending, 0))?.is_empty());
    assert_eq!(
        rows(b"one", config(SortMode::Text, SortDirection::Ascending, 0))?,
        vec![0]
    );
    assert_eq!(
        rows(
            b"\xff\n\x80\na\n",
            config(SortMode::Text, SortDirection::Ascending, 0)
        )?,
        vec![2, 1, 0]
    );
    Ok(())
}

#[test]
fn cancellation_and_quota_release_all_reservations() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let resources = SortResources::new(16 * 1024 * 1024, 128);
    let token = CancellationToken::new();
    let generation = token.snapshot();
    let mut session = SortSession::new(
        Fixture(b"z\na\nb\n".to_vec()),
        config(SortMode::Text, SortDirection::Ascending, 0),
        resources.clone(),
        &scratch.0,
    )?;
    token.cancel();
    assert_eq!(
        session.advance(&token, generation)?.phase,
        SortPhase::Cancelled
    );
    drop(session);
    assert_eq!(resources.memory_bytes(), 0);
    assert_eq!(resources.disk_bytes(), 0);
    let mut session = SortSession::new(
        Fixture(b"z\na\nb\n".to_vec()),
        config(SortMode::Text, SortDirection::Ascending, 0),
        resources.clone(),
        &scratch.0,
    )?;
    let error = loop {
        if let Err(e) = session.advance(&token, token.snapshot()) {
            break e;
        }
    };
    assert!(matches!(error, leanrows_core::SortError::Quota));
    drop(session);
    assert_eq!(resources.disk_bytes(), 0);
    assert!(fs::read_dir(&scratch.0)?.next().is_none());
    Ok(())
}

#[test]
fn incomplete_exponents_are_text_and_decimals_are_exact() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        rows(
            b"1e \n1e+ \n.01\n-.2\n2e-3\n1.\n  +000.01 \n",
            config(SortMode::Numeric, SortDirection::Ascending, 0)
        )?,
        vec![3, 4, 2, 6, 5, 0, 1]
    );
    Ok(())
}

#[test]
fn numeric_permutation_matches_independent_integer_oracle() -> Result<(), Box<dyn Error>> {
    let numbers: Vec<i64> = (0..257).map(|i| ((i * 73 + 19) % 257) - 128).collect();
    let mut input = String::new();
    for number in &numbers {
        writeln!(&mut input, "{number}")?;
    }
    let mut oracle: Vec<usize> = (0..numbers.len()).collect();
    oracle.sort_by_key(|i| numbers[*i]);
    let expected: Vec<u64> = oracle
        .iter()
        .map(|i| u64::try_from(*i))
        .collect::<Result<_, _>>()?;
    assert_eq!(
        rows(
            input.as_bytes(),
            config(SortMode::Numeric, SortDirection::Ascending, 0)
        )?,
        expected
    );
    Ok(())
}

#[test]
fn cancellation_in_every_phase_leaves_no_scratch() -> Result<(), Box<dyn Error>> {
    for phase in [
        SortPhase::Extracting,
        SortPhase::SortingRuns,
        SortPhase::Merging,
        SortPhase::BuildingInverse,
    ] {
        let scratch = Scratch::new()?;
        let resources = SortResources::new(16 * 1024 * 1024, 1024 * 1024);
        let mut session = SortSession::new(
            Fixture(b"z\na\ny\nb\nx\nc\n".to_vec()),
            config(SortMode::Text, SortDirection::Ascending, 0),
            resources.clone(),
            &scratch.0,
        )?;
        let token = CancellationToken::new();
        let generation = token.snapshot();
        while session.progress().phase != phase {
            assert_ne!(
                session.advance(&token, generation)?.phase,
                SortPhase::Complete
            );
        }
        token.cancel();
        assert_eq!(
            session.advance(&token, generation)?.phase,
            SortPhase::Cancelled
        );
        drop(session);
        assert_eq!(resources.disk_bytes(), 0);
        assert_eq!(resources.memory_bytes(), 0);
        assert!(fs::read_dir(&scratch.0)?.next().is_none());
    }
    Ok(())
}

#[test]
fn nonnumeric_suffix_does_not_trigger_numeric_exponent_overflow() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        rows(
            b"1e9223372036854775808x\n2\n",
            config(SortMode::Numeric, SortDirection::Ascending, 0)
        )?,
        vec![1, 0]
    );
    assert!(
        rows(
            b"1e9223372036854775808\n2\n",
            config(SortMode::Numeric, SortDirection::Ascending, 0)
        )
        .is_err()
    );
    Ok(())
}
