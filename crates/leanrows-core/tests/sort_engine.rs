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

// A crash can leave a stale sort folder that another program still holds
// open. Recovery must skip it and keep cleaning, not fail: the window opens
// files only after this recovery has run.
#[cfg(windows)]
#[test]
fn stale_recovery_skips_a_folder_it_cannot_delete() -> Result<(), Box<dyn Error>> {
    use std::os::windows::fs::OpenOptionsExt;
    let scratch = Scratch::new()?;
    let held = scratch.0.join("leanrows-sort-v1-1-0");
    let free = scratch.0.join("leanrows-sort-v1-2-0");
    for dir in [&held, &free] {
        fs::create_dir(dir)?;
        fs::write(dir.join("owner.lock"), b"")?;
        fs::write(dir.join("runs-0.bin"), b"run")?;
    }
    // No sharing at all, as a scanner or backup tool might hold it.
    let handle = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(held.join("runs-0.bin"))?;
    let removed = leanrows_core::recover_stale_sort_files(&scratch.0)?;
    assert_eq!(removed, 1);
    assert!(held.exists(), "the held folder stays for a later recovery");
    assert!(!free.exists(), "recovery continued past the held folder");
    drop(handle);
    assert_eq!(leanrows_core::recover_stale_sort_files(&scratch.0)?, 1);
    assert!(!held.exists());
    Ok(())
}

fn run_to_completion(
    input: &[u8],
    config: SortConfig,
    resources: &std::sync::Arc<SortResources>,
) -> Result<(Vec<u64>, u64), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let mut session = SortSession::new(
        Fixture(input.to_vec()),
        config,
        resources.clone(),
        &scratch.0,
    )?;
    let token = CancellationToken::new();
    let passes = loop {
        let step = session.advance(&token, token.snapshot())?;
        if step.phase == SortPhase::Complete {
            break step.merge_passes;
        }
    };
    let order = session.into_order()?;
    let mut result = Vec::new();
    for rank in 0..order.len() {
        let row = order.record_at(rank)?.source_row;
        assert_eq!(order.rank_of(row)?, rank);
        result.push(row);
    }
    drop(order);
    assert_eq!(resources.disk_bytes(), 0);
    assert_eq!(resources.memory_bytes(), 0);
    Ok((result, passes))
}

/// Rows whose first field is a key with a long shared prefix, so most
/// comparisons must read past the 32 stored bytes, and with repeated keys,
/// so ties must keep source order.
fn shuffled_long_keys(count: u64) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut input = Vec::new();
    let mut keys = Vec::new();
    for row in 0..count {
        let value = (row * 37 + 11) % (count / 2 + 1);
        let key = if row % 3 == 0 {
            format!("{value:03}")
        } else {
            format!("{}{value:05}", "p".repeat(40))
        };
        input.extend_from_slice(key.as_bytes());
        input.extend_from_slice(format!(",row {row}\n").as_bytes());
        keys.push(key.into_bytes());
    }
    (input, keys)
}

fn oracle(keys: &[Vec<u8>], direction: SortDirection) -> Vec<u64> {
    let mut order: Vec<u64> = (0..keys.len() as u64).collect();
    order.sort_by(|a, b| {
        let by_key = keys[usize::try_from(*a).unwrap_or_default()]
            .cmp(&keys[usize::try_from(*b).unwrap_or_default()]);
        let by_key = if direction == SortDirection::Descending {
            by_key.reverse()
        } else {
            by_key
        };
        by_key.then(a.cmp(b))
    });
    order
}

#[test]
fn merge_uses_the_configured_fan_in() -> Result<(), Box<dyn Error>> {
    // 64 rows in runs of 2 make 32 runs. Merging 4 at a time needs
    // 3 passes (4, 16, 64 runs' worth); merging 2 at a time needs 5.
    let (input, keys) = shuffled_long_keys(64);
    let resources = SortResources::new(16 * 1024 * 1024, 8 * 1024 * 1024 * 1024);
    let mut four_way = config(SortMode::Text, SortDirection::Ascending, 0);
    four_way.merge_fan_in = 4;
    let (rows, passes) = run_to_completion(&input, four_way, &resources)?;
    assert_eq!(rows, oracle(&keys, SortDirection::Ascending));
    assert_eq!(passes, 3);
    let (rows, passes) = run_to_completion(
        &input,
        config(SortMode::Text, SortDirection::Ascending, 0),
        &resources,
    )?;
    assert_eq!(rows, oracle(&keys, SortDirection::Ascending));
    assert_eq!(passes, 5);
    Ok(())
}

#[test]
fn many_way_merge_matches_an_independent_oracle() -> Result<(), Box<dyn Error>> {
    let resources = SortResources::new(16 * 1024 * 1024, 8 * 1024 * 1024 * 1024);
    for count in [1, 2, 3, 7, 50, 301] {
        let (input, keys) = shuffled_long_keys(count);
        for fan_in in [2, 3, 5, 8, 64] {
            for batch_capacity in [1, 3, 16] {
                for direction in [SortDirection::Ascending, SortDirection::Descending] {
                    let mut config = config(SortMode::Text, direction, 0);
                    config.merge_fan_in = fan_in;
                    config.batch_capacity = batch_capacity;
                    let (rows, _) = run_to_completion(&input, config, &resources)?;
                    assert_eq!(
                        rows,
                        oracle(&keys, direction),
                        "{count} rows, fan-in {fan_in}, batch {batch_capacity}, {direction:?}"
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
fn merge_buffers_count_against_the_memory_limit() -> Result<(), Box<dyn Error>> {
    // Each merged run needs its own 64 KiB read buffer, so 1,000 runs at a
    // time cannot fit in 16 MiB and the sort must refuse before it starts.
    let scratch = Scratch::new()?;
    let resources = SortResources::new(16 * 1024 * 1024, 8 * 1024 * 1024 * 1024);
    let mut wide = config(SortMode::Text, SortDirection::Ascending, 0);
    wide.merge_fan_in = 1_000;
    let refused = SortSession::new(
        Fixture(b"b\na\n".to_vec()),
        wide,
        resources.clone(),
        &scratch.0,
    );
    assert!(matches!(refused, Err(leanrows_core::SortError::Allocation)));
    assert_eq!(resources.memory_bytes(), 0);
    assert_eq!(resources.disk_bytes(), 0);
    let mut fits = config(SortMode::Text, SortDirection::Ascending, 0);
    fits.merge_fan_in = 64;
    let session = SortSession::new(
        Fixture(b"b\na\n".to_vec()),
        fits,
        resources.clone(),
        &scratch.0,
    )?;
    assert!(resources.memory_bytes() >= 64 * 64 * 1024);
    drop(session);
    assert_eq!(resources.memory_bytes(), 0);
    Ok(())
}

#[test]
fn inverse_map_is_written_in_large_pieces() -> Result<(), Box<dyn Error>> {
    // Writing the inverse map in place costs one write request per row,
    // 20,000 here. Written in sequence it needs a few hundred in all.
    let count = 20_000_u64;
    let mut input = String::new();
    let mut keys = Vec::new();
    for row in 0..count {
        let key = format!("{:05}", (row * 7_919) % count);
        writeln!(&mut input, "{key},{row}")?;
        keys.push(key.into_bytes());
    }
    let resources = SortResources::new(16 * 1024 * 1024, 8 * 1024 * 1024 * 1024);
    let mut config = config(SortMode::Text, SortDirection::Ascending, 0);
    config.batch_capacity = 4_096;
    config.merge_fan_in = 64;
    config.step_bytes = 64 * 1024;
    let (rows, _) = run_to_completion(input.as_bytes(), config, &resources)?;
    assert_eq!(rows, oracle(&keys, SortDirection::Ascending));
    let writes = resources.scratch_write_calls();
    assert!(
        writes < 1_000,
        "{writes} scratch write requests for {count} rows"
    );
    Ok(())
}

#[test]
fn building_the_maps_releases_the_merge_buffers() -> Result<(), Box<dyn Error>> {
    // The merge's batch arrays and read pages are not needed once the maps
    // are built, so the map buffers must replace them, not add to them.
    let scratch = Scratch::new()?;
    let resources = SortResources::new(16 * 1024 * 1024, 8 * 1024 * 1024 * 1024);
    let mut input = String::new();
    for row in 0..5_000_u64 {
        writeln!(&mut input, "{:05}", (row * 7_919) % 5_000)?;
    }
    let mut config = config(SortMode::Text, SortDirection::Ascending, 0);
    config.batch_capacity = 1_024;
    config.merge_fan_in = 64;
    config.step_bytes = 4 * 1024;
    let mut session = SortSession::new(
        Fixture(input.into_bytes()),
        config,
        resources.clone(),
        &scratch.0,
    )?;
    let token = CancellationToken::new();
    while session.advance(&token, token.snapshot())?.phase != SortPhase::BuildingInverse {}
    let held = resources.memory_bytes();
    assert!(
        held < 1024 * 1024,
        "{held} bytes held while building the maps"
    );
    while session.advance(&token, token.snapshot())?.phase != SortPhase::Complete {}
    Ok(())
}

#[test]
fn short_keys_are_compared_without_reading_the_source() -> Result<(), Box<dyn Error>> {
    // Keys of up to 32 bytes are stored whole, and numbers keep their sign,
    // magnitude and digits, so no comparison needs to read the full keys.
    let resources = SortResources::new(16 * 1024 * 1024, 8 * 1024 * 1024 * 1024);
    let (input, keys) = shuffled_short_keys(300);
    let mut text = config(SortMode::Text, SortDirection::Ascending, 0);
    text.merge_fan_in = 4;
    text.batch_capacity = 7;
    let (rows, _) = run_to_completion(&input, text, &resources)?;
    assert_eq!(rows, oracle(&keys, SortDirection::Ascending));
    let mut numeric = text;
    numeric.request.mode = SortMode::Numeric;
    numeric.request.direction = SortDirection::Descending;
    run_to_completion(&input, numeric, &resources)?;
    assert_eq!(resources.key_readers_built(), 0);
    // Long keys that share their first 32 bytes still read the source.
    let (input, keys) = shuffled_long_keys(64);
    let (rows, _) = run_to_completion(&input, text, &resources)?;
    assert_eq!(rows, oracle(&keys, SortDirection::Ascending));
    assert!(resources.key_readers_built() > 0);
    Ok(())
}

fn shuffled_short_keys(count: u64) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut input = Vec::new();
    let mut keys = Vec::new();
    for row in 0..count {
        let key = format!("{}", (row * 37 + 11) % (count / 3 + 1));
        input.extend_from_slice(key.as_bytes());
        input.extend_from_slice(format!(",row {row}\n").as_bytes());
        keys.push(key.into_bytes());
    }
    (input, keys)
}
