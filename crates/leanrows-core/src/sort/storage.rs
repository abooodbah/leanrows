use super::{SortError, key::Descriptor};
use crate::RecordSpan;
use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
const HEADER: u64 = 32;
const MAGIC: &[u8; 8] = b"LRSORT01";
static NEXT: AtomicU64 = AtomicU64::new(0);
#[derive(Debug)]
struct PendingCleanup {
    path: PathBuf,
    disk_bytes: u64,
    memory_bytes: u64,
}
#[derive(Debug)]
pub struct SortResources {
    memory_limit: u64,
    disk_limit: u64,
    memory: AtomicU64,
    disk: AtomicU64,
    peak: AtomicU64,
    disk_peak: AtomicU64,
    pending: Mutex<[Option<PendingCleanup>; 64]>,
}
impl SortResources {
    /// Reserves caller-owned buffers before constructing a sort source adapter.
    pub fn reserve_memory(
        self: &Arc<Self>,
        bytes: usize,
    ) -> Result<SortMemoryReservation, SortError> {
        Ok(SortMemoryReservation {
            _reservation: self.reserve(bytes as u64, true)?,
        })
    }
    #[must_use]
    pub fn new(memory_limit: usize, disk_limit: u64) -> Arc<Self> {
        Arc::new(Self {
            memory_limit: memory_limit as u64,
            disk_limit,
            memory: AtomicU64::new(0),
            disk: AtomicU64::new(0),
            peak: AtomicU64::new(0),
            disk_peak: AtomicU64::new(0),
            pending: Mutex::new(std::array::from_fn(|_| None)),
        })
    }
    pub(super) fn reserve(
        self: &Arc<Self>,
        bytes: u64,
        memory: bool,
    ) -> Result<Reservation, SortError> {
        let counter = if memory { &self.memory } else { &self.disk };
        let limit = if memory {
            self.memory_limit
        } else {
            self.disk_limit
        };
        counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                n.checked_add(bytes).filter(|next| *next <= limit)
            })
            .map_err(|_| {
                if memory {
                    SortError::Allocation
                } else {
                    SortError::Quota
                }
            })?;
        if memory {
            self.peak
                .fetch_max(self.memory.load(Ordering::Acquire), Ordering::Relaxed);
        } else {
            self.disk_peak
                .fetch_max(self.disk.load(Ordering::Acquire), Ordering::Relaxed);
        }
        Ok(Reservation {
            resources: self.clone(),
            bytes,
            memory,
        })
    }
    #[must_use]
    pub fn memory_peak(&self) -> u64 {
        self.peak.load(Ordering::Acquire)
    }
    #[must_use]
    pub fn memory_bytes(&self) -> u64 {
        self.memory.load(Ordering::Acquire)
    }
    #[must_use]
    pub fn disk_bytes(&self) -> u64 {
        self.disk.load(Ordering::Acquire)
    }
    #[must_use]
    pub fn disk_peak(&self) -> u64 {
        self.disk_peak.load(Ordering::Acquire)
    }
    /// Retries a bounded set of owner directories whose deletion was blocked.
    pub fn retry_cleanup(&self) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for slot in pending.iter_mut() {
            let Some(entry) = slot else { continue };
            let deleted = fs::remove_dir_all(&entry.path);
            if deleted.is_ok() || deleted.is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
                self.disk.fetch_sub(entry.disk_bytes, Ordering::AcqRel);
                self.memory.fetch_sub(entry.memory_bytes, Ordering::AcqRel);
                *slot = None;
            }
        }
    }
    fn defer_cleanup(self: &Arc<Self>, path: PathBuf, disk_bytes: u64) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(slot) = pending.iter_mut().find(|slot| slot.is_none()) else {
            return;
        };
        let bytes = path.capacity() as u64 + std::mem::size_of::<PendingCleanup>() as u64;
        let Ok(mut memory) = self.reserve(bytes, true) else {
            return;
        };
        memory.bytes = 0;
        *slot = Some(PendingCleanup {
            path,
            disk_bytes,
            memory_bytes: bytes,
        });
        // If tracking is full or memory is exhausted, the disk charge remains
        // conservative until process exit; the next startup recovers the owner.
    }
}
pub(super) struct Reservation {
    resources: Arc<SortResources>,
    bytes: u64,
    memory: bool,
}
/// Opaque memory lease released after the caller-owned buffer is destroyed.
pub struct SortMemoryReservation {
    _reservation: Reservation,
}
impl Reservation {
    pub fn grow(&mut self, count: u64) -> Result<(), SortError> {
        let mut addition = self.resources.reserve(count, self.memory)?;
        self.bytes += addition.bytes;
        addition.bytes = 0;
        Ok(())
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let counter = if self.memory {
            &self.resources.memory
        } else {
            &self.resources.disk
        };
        counter.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
pub(super) struct Owner {
    pub path: PathBuf,
    lock: Option<File>,
    resources: Arc<SortResources>,
    retained_bytes: AtomicU64,
}
fn exclusive_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    options
}
fn ordinary_directory(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    true
}
impl Owner {
    pub fn create(
        directory: &Path,
        resources: &Arc<SortResources>,
    ) -> Result<Arc<Self>, SortError> {
        if !ordinary_directory(directory) {
            return Err(SortError::InvalidScratch);
        }
        for _ in 0..64 {
            let name = format!(
                "leanrows-sort-v1-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let path = directory.join(name);
            match fs::create_dir(&path) {
                Ok(()) => {
                    let lock = exclusive_options()
                        .create_new(true)
                        .open(path.join("owner.lock"))?;
                    return Ok(Arc::new(Self {
                        path,
                        lock: Some(lock),
                        resources: resources.clone(),
                        retained_bytes: AtomicU64::new(0),
                    }));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
        }
        Err(SortError::InvalidScratch)
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.lock.take();
        let bytes = self.retained_bytes.load(Ordering::Acquire);
        if fs::remove_dir_all(&self.path).is_ok() {
            self.resources.disk.fetch_sub(bytes, Ordering::AcqRel);
        } else if bytes != 0 {
            self.resources
                .defer_cleanup(std::mem::take(&mut self.path), bytes);
        }
    }
}
/// Removes only inactive, versioned sort owner directories.
pub fn recover_stale_sort_files(directory: &Path) -> Result<u64, SortError> {
    if !ordinary_directory(directory) {
        return Err(SortError::InvalidScratch);
    }
    let mut removed = 0;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("leanrows-sort-v1-")
            || !ordinary_directory(&entry.path())
        {
            continue;
        }
        let path = entry.path();
        let Ok(lock) = exclusive_options().open(path.join("owner.lock")) else {
            continue;
        };
        // Windows sharing prevents opening an active owner; do not unlink its lock.
        #[cfg(not(windows))]
        {
            drop(lock);
            continue;
        }
        #[cfg(windows)]
        {
            drop(lock);
            fs::remove_dir_all(path)?;
            removed += 1;
        }
    }
    Ok(removed)
}
pub(super) struct DiskFile {
    pub file: File,
    path: PathBuf,
    pub bytes: u64,
    reservation: Reservation,
    owner: Arc<Owner>,
    writer: Option<BufWriter<File>>,
    buffer_memory: Option<Reservation>,
}
impl DiskFile {
    pub fn create(
        owner: &Arc<Owner>,
        resources: &Arc<SortResources>,
        name: &str,
    ) -> Result<Self, SortError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(owner.path.join(name))?;
        let memory = resources.reserve(64 * 1024, true)?;
        let writer = BufWriter::with_capacity(64 * 1024, file.try_clone()?);
        Ok(Self {
            file,
            path: owner.path.join(name),
            bytes: 0,
            reservation: resources.reserve(0, false)?,
            owner: owner.clone(),
            writer: Some(writer),
            buffer_memory: Some(memory),
        })
    }
    pub fn append(&mut self, bytes: &[u8]) -> Result<(), SortError> {
        self.reservation.grow(bytes.len() as u64)?;
        self.bytes += bytes.len() as u64;
        if let Some(writer) = &mut self.writer {
            writer.write_all(bytes)?;
        } else {
            self.file.write_all(bytes)?;
        }
        Ok(())
    }
    pub fn allocate(&mut self, bytes: u64) -> Result<(), SortError> {
        self.finish_writing()?;
        self.reservation.grow(bytes)?;
        self.bytes += bytes;
        self.file.set_len(self.bytes)?;
        Ok(())
    }
    pub fn finish_writing(&mut self) -> Result<(), SortError> {
        if let Some(writer) = &mut self.writer {
            writer.flush()?;
        }
        self.writer = None;
        self.buffer_memory = None;
        Ok(())
    }
}
impl Drop for DiskFile {
    fn drop(&mut self) {
        self.writer = None;
        // Default sharing allows unlinking our open handle. Other applications
        // can block deletion; transfer its charge to the owner until cleanup.
        if fs::remove_file(&self.path).is_err() {
            self.owner
                .retained_bytes
                .fetch_add(self.reservation.bytes, Ordering::AcqRel);
            self.reservation.bytes = 0;
        }
    }
}
/// A source record at a position in the sorted display order.
#[derive(Clone, Copy, Debug)]
pub struct SortedRecord {
    pub source_row: u64,
    pub span: RecordSpan,
}
pub struct SortedOrder {
    pub(super) order: DiskFile,
    pub(super) inverse: DiskFile,
    pub(super) count: u64,
    pub(super) source_size: u64,
    pub(super) memory: Reservation,
}
impl SortedOrder {
    #[must_use]
    pub fn len(&self) -> u64 {
        self.count
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub fn record_at(&self, rank: u64) -> Result<SortedRecord, SortError> {
        self.validate(&self.order, 24)?;
        if rank >= self.count {
            return Err(SortError::InvalidScratch);
        }
        let mut b = [0; 24];
        read_exact_at(
            &self.order.file,
            HEADER + rank.checked_mul(24).ok_or(SortError::InvalidScratch)?,
            &mut b,
        )?;
        let row = u64::from_le_bytes(b[..8].try_into().map_err(|_| SortError::InvalidScratch)?);
        let start = u64::from_le_bytes(b[8..16].try_into().map_err(|_| SortError::InvalidScratch)?);
        let end = u64::from_le_bytes(
            b[16..24]
                .try_into()
                .map_err(|_| SortError::InvalidScratch)?,
        );
        if row >= self.count || end > self.source_size {
            return Err(SortError::InvalidScratch);
        }
        Ok(SortedRecord {
            source_row: row,
            span: RecordSpan::new(start, end).map_err(|_| SortError::InvalidScratch)?,
        })
    }
    pub fn rank_of(&self, row: u64) -> Result<u64, SortError> {
        self.validate(&self.inverse, 8)?;
        if row >= self.count {
            return Err(SortError::InvalidScratch);
        }
        let mut b = [0; 8];
        read_exact_at(
            &self.inverse.file,
            HEADER + row.checked_mul(8).ok_or(SortError::InvalidScratch)?,
            &mut b,
        )?;
        let rank = u64::from_le_bytes(b);
        if rank >= self.count {
            return Err(SortError::InvalidScratch);
        }
        Ok(rank)
    }
    #[must_use]
    pub fn managed_bytes(&self) -> u64 {
        self.memory.bytes
    }
    fn validate(&self, file: &DiskFile, width: u64) -> Result<(), SortError> {
        let mut header = [0; 32];
        read_exact_at(&file.file, 0, &mut header)?;
        if header != map_header(self.count, self.source_size, width)
            || file.file.metadata()?.len()
                != HEADER
                    + self
                        .count
                        .checked_mul(width)
                        .ok_or(SortError::InvalidScratch)?
        {
            return Err(SortError::InvalidScratch);
        }
        Ok(())
    }
}
pub(super) fn map_header(count: u64, size: u64, width: u64) -> [u8; 32] {
    let mut b = [0; 32];
    b[..8].copy_from_slice(MAGIC);
    b[8..16].copy_from_slice(&count.to_le_bytes());
    b[16..24].copy_from_slice(&size.to_le_bytes());
    b[24..32].copy_from_slice(&width.to_le_bytes());
    b
}
#[cfg(windows)]
pub(super) fn read_exact_at(
    file: &File,
    mut offset: u64,
    mut out: &mut [u8],
) -> Result<(), SortError> {
    use std::os::windows::fs::FileExt;
    while !out.is_empty() {
        let n = file.seek_read(out, offset)?;
        if n == 0 {
            return Err(SortError::InvalidScratch);
        }
        offset += n as u64;
        out = &mut out[n..];
    }
    Ok(())
}
#[cfg(not(windows))]
pub(super) fn read_exact_at(
    file: &File,
    mut offset: u64,
    mut out: &mut [u8],
) -> Result<(), SortError> {
    use std::os::unix::fs::FileExt;
    while !out.is_empty() {
        let n = file.read_at(out, offset)?;
        if n == 0 {
            return Err(SortError::InvalidScratch);
        }
        offset += n as u64;
        out = &mut out[n..];
    }
    Ok(())
}
pub(super) fn encode(d: Descriptor) -> [u8; 128] {
    let mut b = [0; 128];
    for (index, value) in [d.row, d.start, d.end, d.key_start, d.key_end]
        .iter()
        .enumerate()
    {
        b[index * 8..index * 8 + 8].copy_from_slice(&value.to_le_bytes());
    }
    b[40..72].copy_from_slice(&d.prefix);
    b[72..80].copy_from_slice(&d.len.to_le_bytes());
    b[80..96].copy_from_slice(&d.magnitude.to_le_bytes());
    b[96..104].copy_from_slice(&d.leading.to_le_bytes());
    b[104..112].copy_from_slice(&d.digits.to_le_bytes());
    b[112] = d.group;
    b[113] = d.sign.to_le_bytes()[0];
    b[114] = u8::from(d.quoted);
    b
}

pub(super) struct ReadPage {
    pub base: u64,
    bytes: Vec<u8>,
    count: usize,
}
impl ReadPage {
    pub fn new() -> Result<Self, SortError> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(64 * 1024)
            .map_err(|_| SortError::Allocation)?;
        bytes.resize(64 * 1024, 0);
        Ok(Self {
            base: u64::MAX,
            bytes,
            count: 0,
        })
    }
    pub fn record(&mut self, file: &DiskFile, index: u64) -> Result<Descriptor, SortError> {
        let offset = index.checked_mul(128).ok_or(SortError::InvalidScratch)?;
        let base = offset / 65536 * 65536;
        if base != self.base {
            self.count = usize::try_from(
                file.bytes
                    .checked_sub(base)
                    .ok_or(SortError::InvalidScratch)?,
            )
            .unwrap_or(usize::MAX)
            .min(self.bytes.len());
            read_exact_at(&file.file, base, &mut self.bytes[..self.count])?;
            self.base = base;
        }
        let start = usize::try_from(offset - base).map_err(|_| SortError::InvalidScratch)?;
        let bytes = self
            .bytes
            .get(start..start + 128)
            .filter(|_| start + 128 <= self.count)
            .ok_or(SortError::InvalidScratch)?;
        decode(bytes.try_into().map_err(|_| SortError::InvalidScratch)?)
    }
}
pub(super) fn decode(b: &[u8; 128]) -> Result<Descriptor, SortError> {
    fn u(b: &[u8], start: usize) -> Result<u64, SortError> {
        Ok(u64::from_le_bytes(
            b[start..start + 8]
                .try_into()
                .map_err(|_| SortError::InvalidScratch)?,
        ))
    }
    let d = Descriptor {
        row: u(b, 0)?,
        start: u(b, 8)?,
        end: u(b, 16)?,
        key_start: u(b, 24)?,
        key_end: u(b, 32)?,
        prefix: b[40..72]
            .try_into()
            .map_err(|_| SortError::InvalidScratch)?,
        len: u(b, 72)?,
        magnitude: i128::from_le_bytes(
            b[80..96]
                .try_into()
                .map_err(|_| SortError::InvalidScratch)?,
        ),
        leading: u(b, 96)?,
        digits: u(b, 104)?,
        group: b[112],
        sign: i8::from_le_bytes([b[113]]),
        quoted: b[114] == 1,
    };
    if d.start > d.key_start
        || d.key_start > d.key_end
        || d.key_end > d.end
        || d.group > 2
        || !(-1..=1).contains(&d.sign)
        || b[114] > 1
    {
        return Err(SortError::InvalidScratch);
    }
    Ok(d)
}
