use super::key::{Builder, Comparison, Descriptor, Locator, checked_read};
use super::storage::{DiskFile, Owner, ReadPage, Reservation, encode, map_header};
use super::{SortConfig, SortError, SortPhase, SortResources, SortStep, SortedOrder};
use crate::{
    CancellationGeneration, CancellationToken, CsvBoundaryScanner, EmitControl,
    LineBoundaryScanner, PositionedRead, RecordSpan, SourceFingerprint, ViewportFormat,
};
use std::{
    cmp::Ordering,
    io::{Seek, SeekFrom, Write},
    path::Path,
    sync::Arc,
};
enum Scanner {
    Csv(CsvBoundaryScanner),
    Lines(LineBoundaryScanner),
}
impl Scanner {
    fn offset(&self) -> u64 {
        match self {
            Self::Csv(s) => s.offset(),
            Self::Lines(s) => s.offset(),
        }
    }
    fn feed(&mut self, bytes: &[u8]) -> Result<(usize, Option<RecordSpan>), SortError> {
        let mut span = None;
        let mut emit = |s| {
            span = Some(s);
            EmitControl::Stop
        };
        let p = match self {
            Self::Csv(s) => s.feed(bytes, &mut emit)?,
            Self::Lines(s) => s.feed(bytes, &mut emit)?,
        };
        Ok((p.consumed(), span))
    }
    fn finish(self) -> Option<RecordSpan> {
        let mut span = None;
        let mut emit = |s| {
            span = Some(s);
            EmitControl::Stop
        };
        match self {
            Self::Csv(s) => {
                s.finish(&mut emit);
            }
            Self::Lines(s) => {
                s.finish(&mut emit);
            }
        }
        span
    }
}
// One fixed-size decoding buffer avoids a heap allocation for every source row.
#[allow(clippy::large_enum_variant)]
enum Pending {
    Locate(Locator),
    Build(Builder),
}
#[derive(Default)]
struct Merge {
    width: usize,
    start: usize,
    left: usize,
    right: usize,
    middle: usize,
    end: usize,
    out: usize,
}
pub struct SortSession<R> {
    source: R,
    snapshot: SourceFingerprint,
    config: SortConfig,
    resources: Arc<SortResources>,
    owner: Arc<Owner>,
    _memory: Reservation,
    phase: SortPhase,
    scanner: Option<Scanner>,
    scan_buffer: Vec<u8>,
    scan_pos: usize,
    scan_count: usize,
    pending: Option<Pending>,
    batch: Vec<Descriptor>,
    alternate: Vec<Descriptor>,
    merge: Merge,
    comparison: Option<Comparison>,
    write_at: usize,
    records: u64,
    run_file: Option<DiskFile>,
    pass_file: Option<DiskFile>,
    run_width: u64,
    pass: u64,
    group_start: u64,
    left: u64,
    right: u64,
    middle: u64,
    end: u64,
    a: Option<Descriptor>,
    b: Option<Descriptor>,
    order: Option<DiskFile>,
    inverse: Option<DiskFile>,
    inverse_at: u64,
    pages: [ReadPage; 2],
}
impl<R: PositionedRead> SortSession<R> {
    pub fn new(
        source: R,
        config: SortConfig,
        resources: Arc<SortResources>,
        directory: &Path,
    ) -> Result<Self, SortError> {
        if config.batch_capacity == 0 || config.merge_fan_in < 2 || config.step_bytes == 0 {
            return Err(SortError::InvalidConfig);
        }
        let snapshot = SourceFingerprint::capture(&source)?;
        let bytes = config
            .batch_capacity
            .checked_mul(std::mem::size_of::<Descriptor>())
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(512 * 1024))
            .ok_or(SortError::Allocation)?;
        let memory = resources.reserve(bytes as u64, true)?;
        let mut batch = Vec::new();
        let mut alternate = Vec::new();
        let mut scan_buffer = Vec::new();
        batch
            .try_reserve_exact(config.batch_capacity)
            .map_err(|_| SortError::Allocation)?;
        alternate
            .try_reserve_exact(config.batch_capacity)
            .map_err(|_| SortError::Allocation)?;
        alternate.resize(config.batch_capacity, Descriptor::default());
        scan_buffer
            .try_reserve_exact(64 * 1024)
            .map_err(|_| SortError::Allocation)?;
        scan_buffer.resize(64 * 1024, 0);
        resources.retry_cleanup();
        let owner = Owner::create(directory, &resources)?;
        let run_file = DiskFile::create(&owner, &resources, "runs-0.bin")?;
        let scanner = match config.format {
            ViewportFormat::Delimited(d) => Scanner::Csv(CsvBoundaryScanner::new(d)),
            ViewportFormat::Lines => Scanner::Lines(LineBoundaryScanner::new()),
        };
        Ok(Self {
            source,
            snapshot,
            config,
            resources,
            owner,
            _memory: memory,
            phase: SortPhase::Extracting,
            scanner: Some(scanner),
            scan_buffer,
            scan_pos: 0,
            scan_count: 0,
            pending: None,
            batch,
            alternate,
            merge: Merge::default(),
            comparison: None,
            write_at: 0,
            records: 0,
            run_file: Some(run_file),
            pass_file: None,
            run_width: config.batch_capacity as u64,
            pass: 0,
            group_start: 0,
            left: 0,
            right: 0,
            middle: 0,
            end: 0,
            a: None,
            b: None,
            order: None,
            inverse: None,
            inverse_at: 0,
            pages: [ReadPage::new()?, ReadPage::new()?],
        })
    }
    fn quote(&self) -> u8 {
        match self.config.format {
            ViewportFormat::Delimited(d) => d.quote(),
            ViewportFormat::Lines => b'"',
        }
    }
    pub fn advance(
        &mut self,
        token: &CancellationToken,
        generation: CancellationGeneration,
    ) -> Result<SortStep, SortError> {
        if token.is_cancelled(generation) {
            self.phase = SortPhase::Cancelled;
            return Ok(self.progress());
        }
        if self
            .snapshot
            .compare(SourceFingerprint::capture(&self.source)?)
            .is_changed()
        {
            return Err(SortError::SourceChanged);
        }
        let mut budget = self.config.step_bytes;
        while budget > 0 && !matches!(self.phase, SortPhase::Complete | SortPhase::Cancelled) {
            if token.is_cancelled(generation) {
                self.phase = SortPhase::Cancelled;
                break;
            }
            match self.phase {
                SortPhase::Extracting => self.extract(&mut budget)?,
                SortPhase::SortingRuns => self.sort_batch(&mut budget)?,
                SortPhase::Merging => self.merge_disk(&mut budget)?,
                SortPhase::BuildingInverse => self.build_maps(&mut budget)?,
                SortPhase::Complete | SortPhase::Cancelled => {}
            }
        }
        if self
            .snapshot
            .compare(SourceFingerprint::capture(&self.source)?)
            .is_changed()
        {
            return Err(SortError::SourceChanged);
        }
        Ok(self.progress())
    }
    #[must_use]
    pub fn progress(&self) -> SortStep {
        SortStep {
            phase: self.phase,
            scanned_bytes: self
                .scanner
                .as_ref()
                .map_or(self.snapshot.size(), Scanner::offset),
            source_bytes: self.snapshot.size(),
            records: self.records,
        }
    }
    pub fn into_order(mut self) -> Result<SortedOrder, SortError> {
        if self.phase != SortPhase::Complete {
            return Err(SortError::InvalidConfig);
        }
        if SourceFingerprint::capture(&self.source)? != self.snapshot {
            return Err(SortError::SourceChanged);
        }
        Ok(SortedOrder {
            order: self.order.take().ok_or(SortError::InvalidScratch)?,
            inverse: self.inverse.take().ok_or(SortError::InvalidScratch)?,
            count: self.records,
            source_size: self.snapshot.size(),
            memory: self.resources.reserve(4096, true)?,
        })
    }
    fn extract(&mut self, budget: &mut usize) -> Result<(), SortError> {
        if let Some(pending) = &mut self.pending {
            match pending {
                Pending::Locate(locator) => {
                    if let Some(d) = locator.advance(&self.source, budget)? {
                        self.pending = Some(Pending::Build(Builder::new(d, self.quote())));
                    }
                }
                Pending::Build(builder) => {
                    if let Some(d) =
                        builder.advance(&self.source, budget, self.config.request.mode)?
                    {
                        self.batch.push(d);
                        self.records += 1;
                        self.pending = None;
                    }
                }
            }
            return Ok(());
        }
        if self.batch.len() == self.config.batch_capacity || self.scanner.is_none() {
            if self.batch.is_empty() {
                self.begin_disk_merge()?;
            } else {
                self.merge = Merge {
                    width: 1,
                    ..Merge::default()
                };
                self.write_at = 0;
                self.phase = SortPhase::SortingRuns;
            }
            return Ok(());
        }
        let mut span = None;
        if let Some(scanner) = &mut self.scanner {
            if scanner.offset() == self.snapshot.size() {
                span = self.scanner.take().and_then(Scanner::finish);
            } else {
                if self.scan_pos == self.scan_count {
                    let count = self.scan_buffer.len().min(
                        usize::try_from(self.snapshot.size() - scanner.offset())
                            .unwrap_or(usize::MAX),
                    );
                    self.scan_count = checked_read(
                        &self.source,
                        scanner.offset(),
                        &mut self.scan_buffer[..count],
                    )?;
                    self.scan_pos = 0;
                }
                let count = (*budget).min(self.scan_count - self.scan_pos);
                let (consumed, emitted) =
                    scanner.feed(&self.scan_buffer[self.scan_pos..self.scan_pos + count])?;
                self.scan_pos += consumed;
                *budget -= consumed;
                span = emitted;
            }
        }
        if let Some(span) = span {
            self.pending = Some(Pending::Locate(Locator::new(
                self.records,
                span,
                self.config.format,
                self.config.request.column,
            )));
        }
        Ok(())
    }
    fn sort_batch(&mut self, budget: &mut usize) -> Result<(), SortError> {
        let n = self.batch.len();
        if self.merge.width >= n {
            if self.write_at < n {
                self.run_file
                    .as_mut()
                    .ok_or(SortError::InvalidScratch)?
                    .append(&encode(self.batch[self.write_at]))?;
                self.write_at += 1;
                *budget = budget.saturating_sub(128);
            } else {
                self.batch.clear();
                self.phase = SortPhase::Extracting;
            }
            return Ok(());
        }
        if self.merge.start >= n {
            self.batch.copy_from_slice(&self.alternate[..n]);
            self.merge.width = self.merge.width.saturating_mul(2);
            self.merge.start = 0;
            self.merge.end = 0;
            self.merge.out = 0;
            *budget = budget.saturating_sub(n.saturating_mul(128));
            return Ok(());
        }
        if self.merge.out == self.merge.end {
            let start = self.merge.start;
            let middle = start.saturating_add(self.merge.width).min(n);
            let end = middle.saturating_add(self.merge.width).min(n);
            self.merge.left = start;
            self.merge.right = middle;
            self.merge.middle = middle;
            self.merge.end = end;
            self.merge.out = start;
        }
        let take_left = if self.merge.left == self.merge.middle {
            false
        } else if self.merge.right == self.merge.end {
            true
        } else {
            if self.comparison.is_none() {
                self.comparison = Some(Comparison::new(
                    self.batch[self.merge.left],
                    self.batch[self.merge.right],
                    self.config.request,
                    self.quote(),
                ));
            }
            let Some(order) = self
                .comparison
                .as_mut()
                .ok_or(SortError::InvalidScratch)?
                .advance(&self.source, budget)?
            else {
                return Ok(());
            };
            self.comparison = None;
            order != Ordering::Greater
        };
        let index = if take_left {
            let i = self.merge.left;
            self.merge.left += 1;
            i
        } else {
            let i = self.merge.right;
            self.merge.right += 1;
            i
        };
        self.alternate[self.merge.out] = self.batch[index];
        self.merge.out += 1;
        if self.merge.out == self.merge.end {
            self.merge.start = self.merge.end;
        }
        *budget = budget.saturating_sub(128);
        Ok(())
    }
    fn begin_disk_merge(&mut self) -> Result<(), SortError> {
        self.run_file
            .as_mut()
            .ok_or(SortError::InvalidScratch)?
            .finish_writing()?;
        for page in &mut self.pages {
            page.base = u64::MAX;
        }
        if self.records <= self.run_width {
            self.begin_maps()?;
        } else {
            self.pass += 1;
            self.pass_file = Some(DiskFile::create(
                &self.owner,
                &self.resources,
                &format!("runs-{}.bin", self.pass),
            )?);
            self.group_start = 0;
            self.end = 0;
            self.phase = SortPhase::Merging;
        }
        Ok(())
    }
    fn load_descriptor(&mut self, index: u64, page: usize) -> Result<Descriptor, SortError> {
        self.pages[page].record(
            self.run_file.as_ref().ok_or(SortError::InvalidScratch)?,
            index,
        )
    }
    fn merge_disk(&mut self, budget: &mut usize) -> Result<(), SortError> {
        if self.group_start == self.records {
            self.run_file = self.pass_file.take();
            self.run_width = self.run_width.saturating_mul(2);
            self.begin_disk_merge()?;
            return Ok(());
        }
        if self.end == self.group_start {
            self.left = self.group_start;
            self.middle = self
                .group_start
                .saturating_add(self.run_width)
                .min(self.records);
            self.right = self.middle;
            self.end = self.middle.saturating_add(self.run_width).min(self.records);
        }
        if self.a.is_none() && self.left < self.middle {
            self.a = Some(self.load_descriptor(self.left, 0)?);
        }
        if self.b.is_none() && self.right < self.end {
            self.b = Some(self.load_descriptor(self.right, 1)?);
        }
        let left = match (self.a, self.b) {
            (Some(a), Some(b)) => {
                if self.comparison.is_none() {
                    self.comparison =
                        Some(Comparison::new(a, b, self.config.request, self.quote()));
                }
                let Some(o) = self
                    .comparison
                    .as_mut()
                    .ok_or(SortError::InvalidScratch)?
                    .advance(&self.source, budget)?
                else {
                    return Ok(());
                };
                self.comparison = None;
                o != Ordering::Greater
            }
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => return Err(SortError::InvalidScratch),
        };
        let d = if left {
            self.left += 1;
            self.a.take()
        } else {
            self.right += 1;
            self.b.take()
        }
        .ok_or(SortError::InvalidScratch)?;
        self.pass_file
            .as_mut()
            .ok_or(SortError::InvalidScratch)?
            .append(&encode(d))?;
        *budget = budget.saturating_sub(128);
        if self.left == self.middle && self.right == self.end {
            self.group_start = self.end;
        }
        Ok(())
    }
    fn begin_maps(&mut self) -> Result<(), SortError> {
        let mut order = DiskFile::create(&self.owner, &self.resources, "order.bin")?;
        order.append(&map_header(self.records, self.snapshot.size(), 24))?;
        let mut inverse = DiskFile::create(&self.owner, &self.resources, "inverse.bin")?;
        inverse.append(&map_header(self.records, self.snapshot.size(), 8))?;
        inverse.allocate(
            self.records
                .checked_mul(8)
                .ok_or(SortError::InvalidScratch)?,
        )?;
        self.order = Some(order);
        self.inverse = Some(inverse);
        self.phase = SortPhase::BuildingInverse;
        Ok(())
    }
    fn build_maps(&mut self, budget: &mut usize) -> Result<(), SortError> {
        if self.inverse_at == self.records {
            self.order
                .as_mut()
                .ok_or(SortError::InvalidScratch)?
                .finish_writing()?;
            self.run_file = None;
            self.phase = SortPhase::Complete;
            return Ok(());
        }
        let d = self.load_descriptor(self.inverse_at, 0)?;
        let mut bytes = [0; 24];
        bytes[..8].copy_from_slice(&d.row.to_le_bytes());
        bytes[8..16].copy_from_slice(&d.start.to_le_bytes());
        bytes[16..24].copy_from_slice(&d.end.to_le_bytes());
        self.order
            .as_mut()
            .ok_or(SortError::InvalidScratch)?
            .append(&bytes)?;
        let inverse = &mut self.inverse.as_mut().ok_or(SortError::InvalidScratch)?.file;
        inverse.seek(SeekFrom::Start(
            32 + d.row.checked_mul(8).ok_or(SortError::InvalidScratch)?,
        ))?;
        inverse.write_all(&self.inverse_at.to_le_bytes())?;
        self.inverse_at += 1;
        *budget = budget.saturating_sub(128);
        Ok(())
    }
}
