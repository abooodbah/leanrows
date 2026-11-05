use super::inverse::{BUCKET_BUFFER_BYTES, BUCKET_ROWS, InverseBuilder};
use super::key::{Builder, Comparison, Descriptor, Locator, checked_read};
use super::storage::{DiskFile, Owner, ReadPage, Reservation, encode, map_header};
use super::{SortConfig, SortError, SortPhase, SortResources, SortStep, SortedOrder};
use crate::{
    CancellationGeneration, CancellationToken, CsvBoundaryScanner, EmitControl,
    LineBoundaryScanner, PositionedRead, RecordSpan, SourceFingerprint, ViewportFormat,
};
use std::{cmp::Ordering, path::Path, sync::Arc};
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
/// Scan buffer, the first read page, cursors and other fixed state.
const FIXED_BYTES: usize = 512 * 1024;
/// Bytes of one run's read page during a merge.
const PAGE_BYTES: usize = 64 * 1024;
/// Memory charged per merged run for its tree nodes, head and positions.
const TREE_BYTES_PER_RUN: usize = 256;
/// A tree node or leaf with no run, or a run with no records left.
const EMPTY: u32 = u32::MAX;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    /// Between merge groups.
    Idle,
    /// Filling inner nodes from the last one up to the root.
    Build(usize),
    /// Writing the root's record and reading that run's next one.
    Emit,
    /// Replaying the matches from that run's leaf up to the root.
    Replay(usize),
}
/// A winner tree over the runs of one merge group. Leaf `leaves + r` is run
/// `r`; every inner node holds the run whose head sorts first below it.
struct Tree {
    leaves: usize,
    nodes: Vec<u32>,
    heads: Vec<Option<Descriptor>>,
    next: Vec<u64>,
    end: Vec<u64>,
    group_end: u64,
    stage: Stage,
}
impl Tree {
    fn new(fan_in: usize) -> Result<Self, SortError> {
        let leaves = fan_in
            .checked_next_power_of_two()
            .ok_or(SortError::InvalidConfig)?;
        Ok(Self {
            leaves,
            nodes: filled(
                leaves.checked_mul(2).ok_or(SortError::InvalidConfig)?,
                EMPTY,
            )?,
            heads: filled(fan_in, None)?,
            next: filled(fan_in, 0)?,
            end: filled(fan_in, 0)?,
            group_end: 0,
            stage: Stage::Idle,
        })
    }
}
fn filled<T: Clone>(len: usize, value: T) -> Result<Vec<T>, SortError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(len)
        .map_err(|_| SortError::Allocation)?;
    values.resize(len, value);
    Ok(values)
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
    _fixed_memory: Reservation,
    merge_memory: Option<Reservation>,
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
    tree: Tree,
    order: Option<DiskFile>,
    inverse: Option<DiskFile>,
    inverse_at: u64,
    inverse_builder: Option<InverseBuilder>,
    pages: Vec<ReadPage>,
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
        // Fixed buffers stay for the whole sort. The batch and its merge copy,
        // and one read page and tree slot for every run merged at a time, are
        // released before the maps are built.
        let fixed_memory = resources.reserve(FIXED_BYTES as u64, true)?;
        let merge_bytes = config
            .batch_capacity
            .checked_mul(std::mem::size_of::<Descriptor>())
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| {
                n.checked_add(
                    config
                        .merge_fan_in
                        .checked_mul(PAGE_BYTES + TREE_BYTES_PER_RUN)?,
                )
            })
            .ok_or(SortError::Allocation)?;
        let merge_memory = resources.reserve(merge_bytes as u64, true)?;
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
        // Pages for a merge are added when one starts, so a file that fits in
        // one run never allocates them.
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(config.merge_fan_in)
            .map_err(|_| SortError::Allocation)?;
        pages.push(ReadPage::new()?);
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
            _fixed_memory: fixed_memory,
            merge_memory: Some(merge_memory),
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
            tree: Tree::new(config.merge_fan_in)?,
            order: None,
            inverse: None,
            inverse_at: 0,
            inverse_builder: None,
            pages,
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
            merge_passes: self.pass,
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
            let runs = usize::try_from(self.records.div_ceil(self.run_width))
                .unwrap_or(usize::MAX)
                .min(self.config.merge_fan_in);
            while self.pages.len() < runs {
                self.pages.push(ReadPage::new()?);
            }
            self.pass += 1;
            self.pass_file = Some(DiskFile::create(
                &self.owner,
                &self.resources,
                &format!("runs-{}.bin", self.pass),
            )?);
            self.group_start = 0;
            self.tree.stage = Stage::Idle;
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
    /// Merges up to `merge_fan_in` sorted runs at a time through a winner
    /// tree, so one pass reads and writes every record once and the number
    /// of passes is the logarithm of the run count to that base.
    fn merge_disk(&mut self, budget: &mut usize) -> Result<(), SortError> {
        if self.tree.stage == Stage::Idle {
            if self.group_start == self.records {
                self.run_file = self.pass_file.take();
                self.run_width = self
                    .run_width
                    .saturating_mul(self.config.merge_fan_in as u64);
                self.begin_disk_merge()?;
                return Ok(());
            }
            self.start_group()?;
        }
        match self.tree.stage {
            Stage::Build(node) => {
                let Some(winner) = self.play(node, budget)? else {
                    return Ok(());
                };
                self.tree.nodes[node] = winner;
                self.tree.stage = if node > 1 {
                    Stage::Build(node - 1)
                } else {
                    Stage::Emit
                };
            }
            Stage::Emit => {
                let run = self.tree.nodes[1];
                if run == EMPTY {
                    self.group_start = self.tree.group_end;
                    self.tree.stage = Stage::Idle;
                    return Ok(());
                }
                let index = run as usize;
                let d = self.tree.heads[index]
                    .take()
                    .ok_or(SortError::InvalidScratch)?;
                self.pass_file
                    .as_mut()
                    .ok_or(SortError::InvalidScratch)?
                    .append(&encode(d))?;
                *budget = budget.saturating_sub(128);
                self.tree.next[index] += 1;
                self.load_head(index)?;
                let leaf = self.tree.leaves + index;
                self.tree.nodes[leaf] = if self.tree.heads[index].is_some() {
                    run
                } else {
                    EMPTY
                };
                self.tree.stage = Stage::Replay(leaf / 2);
            }
            Stage::Replay(node) => {
                let Some(winner) = self.play(node, budget)? else {
                    return Ok(());
                };
                self.tree.nodes[node] = winner;
                self.tree.stage = if node > 1 {
                    Stage::Replay(node / 2)
                } else {
                    Stage::Emit
                };
            }
            Stage::Idle => {}
        }
        Ok(())
    }
    /// Sets up the next group of runs and reads the first record of each.
    fn start_group(&mut self) -> Result<(), SortError> {
        let mut start = self.group_start;
        self.tree.nodes.fill(EMPTY);
        for run in 0..self.pages.len() {
            let end = start.saturating_add(self.run_width).min(self.records);
            self.tree.next[run] = start;
            self.tree.end[run] = end;
            start = end;
            self.load_head(run)?;
            if self.tree.heads[run].is_some() {
                self.tree.nodes[self.tree.leaves + run] =
                    u32::try_from(run).map_err(|_| SortError::InvalidConfig)?;
            }
        }
        self.tree.group_end = start;
        self.tree.stage = if self.tree.leaves > 1 {
            Stage::Build(self.tree.leaves - 1)
        } else {
            Stage::Emit
        };
        Ok(())
    }
    fn load_head(&mut self, run: usize) -> Result<(), SortError> {
        self.tree.heads[run] = if self.tree.next[run] < self.tree.end[run] {
            Some(self.load_descriptor(self.tree.next[run], run)?)
        } else {
            None
        };
        Ok(())
    }
    /// The winner of one tree node's two children, or `None` when the budget
    /// ran out in the middle of a comparison. Ties cannot occur: equal keys
    /// are ordered by source row.
    fn play(&mut self, node: usize, budget: &mut usize) -> Result<Option<u32>, SortError> {
        let left = self.tree.nodes[node * 2];
        let right = self.tree.nodes[node * 2 + 1];
        if left == EMPTY {
            return Ok(Some(right));
        }
        if right == EMPTY {
            return Ok(Some(left));
        }
        let a = self.tree.heads[left as usize].ok_or(SortError::InvalidScratch)?;
        let b = self.tree.heads[right as usize].ok_or(SortError::InvalidScratch)?;
        if self.comparison.is_none() {
            self.comparison = Some(Comparison::new(a, b, self.config.request, self.quote()));
        }
        let Some(order) = self
            .comparison
            .as_mut()
            .ok_or(SortError::InvalidScratch)?
            .advance(&self.source, budget)?
        else {
            return Ok(None);
        };
        self.comparison = None;
        Ok(Some(if order == Ordering::Greater {
            right
        } else {
            left
        }))
    }
    fn begin_maps(&mut self) -> Result<(), SortError> {
        // The maps read the last run file through page 0 alone; the batch
        // arrays, the other pages and the tree go before the map buffers come.
        self.batch = Vec::new();
        self.alternate = Vec::new();
        self.pages.truncate(1);
        self.pages.shrink_to_fit();
        self.tree = Tree::new(1)?;
        self.merge_memory = None;
        let mut order = DiskFile::create(&self.owner, &self.resources, "order.bin")?;
        order.append(&map_header(self.records, self.snapshot.size(), 24))?;
        let mut inverse = DiskFile::create(&self.owner, &self.resources, "inverse.bin")?;
        inverse.append(&map_header(self.records, self.snapshot.size(), 8))?;
        inverse.allocate(
            self.records
                .checked_mul(8)
                .ok_or(SortError::InvalidScratch)?,
        )?;
        self.inverse_builder = Some(InverseBuilder::new(
            &self.owner,
            &self.resources,
            self.records,
            BUCKET_ROWS,
            BUCKET_BUFFER_BYTES,
        )?);
        self.order = Some(order);
        self.inverse = Some(inverse);
        self.phase = SortPhase::BuildingInverse;
        Ok(())
    }
    /// Writes the order map in rank order while collecting (row, rank)
    /// pairs, then places the pairs into the inverse map bucket by bucket.
    fn build_maps(&mut self, budget: &mut usize) -> Result<(), SortError> {
        if self.inverse_at == self.records {
            let builder = self
                .inverse_builder
                .as_mut()
                .ok_or(SortError::InvalidScratch)?;
            if self.run_file.is_some() {
                builder.finish_collecting()?;
                self.order
                    .as_mut()
                    .ok_or(SortError::InvalidScratch)?
                    .finish_writing()?;
                self.run_file = None;
                return Ok(());
            }
            let inverse = self.inverse.as_ref().ok_or(SortError::InvalidScratch)?;
            if builder.place(inverse, budget)? {
                self.inverse_builder = None;
                self.phase = SortPhase::Complete;
            }
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
        self.inverse_builder
            .as_mut()
            .ok_or(SortError::InvalidScratch)?
            .push(d.row, self.inverse_at)?;
        self.inverse_at += 1;
        *budget = budget.saturating_sub(128);
        Ok(())
    }
}
