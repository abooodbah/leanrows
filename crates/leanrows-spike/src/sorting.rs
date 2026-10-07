use super::{AppError, InputFormat};
use leanrows_core::{
    CancellationToken, FileSource, PositionedRead, SortConfig, SortDirection, SortMode, SortPhase,
    SortRequest, SortResources, SortSession, SourceFingerprint,
};
use std::{
    cell::RefCell,
    ffi::OsString,
    fs, io,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}
struct Cache {
    bytes: Vec<u8>,
    base: u64,
    count: usize,
}
struct CachedSource {
    source: FileSource,
    cache: RefCell<Cache>,
}
impl CachedSource {
    fn new(source: FileSource) -> Result<Self, AppError> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(64 * 1024)
            .map_err(|e| AppError::runtime(e.to_string()))?;
        bytes.resize(64 * 1024, 0);
        Ok(Self {
            source,
            cache: RefCell::new(Cache {
                bytes,
                base: u64::MAX,
                count: 0,
            }),
        })
    }
}
impl PositionedRead for CachedSource {
    fn size(&self) -> io::Result<u64> {
        self.source.size()
    }
    fn fingerprint(&self) -> io::Result<SourceFingerprint> {
        self.source.fingerprint()
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> io::Result<usize> {
        let mut cache = self.cache.borrow_mut();
        let base = offset / 65536 * 65536;
        if cache.base != base {
            cache.count = self.source.read_at(base, &mut cache.bytes)?;
            cache.base = base;
        }
        let start = usize::try_from(offset - base).map_err(io::Error::other)?;
        let count = out.len().min(cache.count.saturating_sub(start));
        if count > 0 {
            out[..count].copy_from_slice(&cache.bytes[start..start + count]);
        }
        Ok(count)
    }
}
#[allow(
    clippy::too_many_lines,
    reason = "one bounded probe parses input, runs the engine and reports its measured resources"
)]
pub(super) fn run(arguments: Vec<OsString>) -> Result<String, AppError> {
    let mut args = arguments.into_iter();
    let _ = args.next();
    let path = PathBuf::from(
        args.next()
            .ok_or_else(|| AppError::usage("--sort requires a file"))?,
    );
    let mut request = SortRequest {
        column: 0,
        mode: SortMode::Text,
        direction: SortDirection::Ascending,
    };
    let mut format = None;
    while let Some(option) = args.next() {
        match option.to_str() {
            Some("--numeric") => request.mode = SortMode::Numeric,
            Some("--descending") => request.direction = SortDirection::Descending,
            Some("--column") => {
                request.column = args
                    .next()
                    .and_then(|s| s.to_str().and_then(|s| s.parse().ok()))
                    .ok_or_else(|| AppError::usage("--column requires a zero-based index"))?;
            }
            Some("--format") => {
                let value = args
                    .next()
                    .ok_or_else(|| AppError::usage("--format requires a value"))?;
                format = Some(InputFormat::parse(
                    value
                        .to_str()
                        .ok_or_else(|| AppError::usage("invalid format"))?,
                )?);
            }
            _ => return Err(AppError::usage("unknown sort probe option")),
        }
    }
    let format = match format {
        Some(f) => f,
        None => InputFormat::infer(&path)?,
    };
    let source = FileSource::open(&path).map_err(|e| AppError::runtime(e.to_string()))?;
    let source_size = source
        .size()
        .map_err(|e| AppError::runtime(e.to_string()))?;
    let source = CachedSource::new(source)?;
    let root = std::env::temp_dir().join(format!(
        "leanrows-sort-probe-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).map_err(|e| AppError::runtime(e.to_string()))?;
    let scratch = Scratch(root);
    let resources = SortResources::new(16 * 1024 * 1024, 8 * 1024 * 1024 * 1024);
    let config = SortConfig {
        format: format.viewport_format(),
        request,
        batch_capacity: 8192,
        merge_fan_in: 2,
        step_bytes: 64 * 1024,
    };
    let token = CancellationToken::new();
    let started = Instant::now();
    let mut session = SortSession::new(source, config, resources.clone(), &scratch.0)
        .map_err(|e| AppError::runtime(e.to_string()))?;
    while session
        .advance(&token, token.snapshot())
        .map_err(|e| AppError::runtime(e.to_string()))?
        .phase
        != SortPhase::Complete
    {}
    let order = session
        .into_order()
        .map_err(|e| AppError::runtime(e.to_string()))?;
    let count = order.len();
    let mut samples = String::from("[");
    for rank in 0..count.min(8) {
        if rank > 0 {
            samples.push(',');
        }
        let row = order
            .record_at(rank)
            .map_err(|e| AppError::runtime(e.to_string()))?
            .source_row;
        samples.push_str(&row.to_string());
        if order
            .rank_of(row)
            .map_err(|e| AppError::runtime(e.to_string()))?
            != rank
        {
            return Err(AppError::runtime("sort inverse map is inconsistent"));
        }
    }
    samples.push(']');
    let output = format!(
        "{{\"schema\":\"leanrows.sort-benchmark\",\"version\":1,\"record_count\":{count},\"source_bytes\":{source_size},\"elapsed_ms\":{},\"managed_peak_bytes\":{},\"scratch_peak_bytes\":{},\"sample_source_rows\":{samples}}}",
        started.elapsed().as_millis(),
        resources.memory_peak(),
        resources.disk_peak()
    );
    drop(order);
    if resources.disk_bytes() != 0 || resources.memory_bytes() != 0 {
        return Err(AppError::runtime("sort resources were not released"));
    }
    Ok(output)
}
