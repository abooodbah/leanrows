//! Read-only ZIP directory parsing and streamed entry decompression.
//!
//! Only what an `.xlsx` package needs is supported: one disk, no ZIP64, no
//! encryption, and the stored (0) or deflate (8) methods. Entry data is
//! decompressed into a fixed 32 KiB window, so memory does not grow with the
//! entry size.

use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};

use super::{XlsxError, XlsxLimits};
use crate::PositionedRead;

const LOCAL_HEADER_SIGNATURE: u32 = 0x0403_4b50;
const CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;
const END_SIGNATURE: u32 = 0x0605_4b50;
const ZIP64_LOCATOR_SIGNATURE: u32 = 0x0706_4b50;
const OLE_SIGNATURE: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
const END_RECORD_BYTES: usize = 22;
const MAX_COMMENT_BYTES: usize = 0xffff;
const LOCAL_HEADER_BYTES: usize = 30;
const CENTRAL_HEADER_BYTES: usize = 46;
const MAX_NAME_BYTES: usize = 1_024;
/// The deflate window. It must be a power of two and at least 32 KiB.
pub(crate) const WINDOW_BYTES: usize = 32 * 1_024;
const INPUT_BYTES: usize = 16 * 1_024;
const METHOD_STORED: u16 = 0;
const METHOD_DEFLATE: u16 = 8;

/// One central-directory record.
#[derive(Clone, Debug)]
pub(crate) struct ZipEntry {
    name: Box<str>,
    method: u16,
    crc32: u32,
    compressed: u64,
    uncompressed: u64,
    local_header: u64,
}

impl ZipEntry {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) const fn compressed_bytes(&self) -> u64 {
        self.compressed
    }
}

/// The entries of one archive, in central-directory order.
#[derive(Debug)]
pub(crate) struct ZipDirectory {
    entries: Vec<ZipEntry>,
    central_directory: u64,
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    match bytes.get(offset..offset + 2) {
        Some(&[a, b]) => u16::from_le_bytes([a, b]),
        _ => 0,
    }
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    match bytes.get(offset..offset + 4) {
        Some(&[a, b, c, d]) => u32::from_le_bytes([a, b, c, d]),
        _ => 0,
    }
}

fn read_exact<R: PositionedRead + ?Sized>(
    source: &R,
    offset: u64,
    destination: &mut [u8],
) -> Result<(), XlsxError> {
    let mut filled = 0;
    while filled < destination.len() {
        let position = offset
            .checked_add(filled as u64)
            .ok_or(XlsxError::MalformedZip("offset overflow"))?;
        let count = source
            .read_at(position, &mut destination[filled..])
            .map_err(XlsxError::Io)?;
        if count == 0 {
            return Err(XlsxError::Truncated);
        }
        if count > destination.len() - filled {
            return Err(XlsxError::MalformedZip("source returned too many bytes"));
        }
        filled += count;
    }
    Ok(())
}

fn zeroed(length: usize) -> Result<Vec<u8>, XlsxError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| XlsxError::AllocationFailed)?;
    bytes.resize(length, 0);
    Ok(bytes)
}

impl ZipDirectory {
    #[allow(
        clippy::too_many_lines,
        reason = "the end record and central directory are checked in file order"
    )]
    pub(crate) fn read<R: PositionedRead + ?Sized>(
        source: &R,
        limits: &XlsxLimits,
    ) -> Result<Self, XlsxError> {
        let size = source.size().map_err(XlsxError::Io)?;
        let mut magic = [0_u8; 8];
        let magic_length = usize::try_from(size.min(8)).unwrap_or(8);
        read_exact(source, 0, &mut magic[..magic_length])?;
        if magic == OLE_SIGNATURE {
            return Err(XlsxError::LegacyOrEncrypted);
        }
        if u32_at(&magic, 0) != LOCAL_HEADER_SIGNATURE {
            return Err(XlsxError::NotZip);
        }
        if size < END_RECORD_BYTES as u64 {
            return Err(XlsxError::Truncated);
        }

        let tail_length = size.min((END_RECORD_BYTES + MAX_COMMENT_BYTES) as u64);
        let tail_start = size - tail_length;
        let mut tail = zeroed(usize::try_from(tail_length).unwrap_or(0))?;
        read_exact(source, tail_start, &mut tail)?;
        let mut end = None;
        let mut position = tail.len() - END_RECORD_BYTES;
        loop {
            if u32_at(&tail, position) == END_SIGNATURE {
                let comment = usize::from(u16_at(&tail, position + 20));
                if position + END_RECORD_BYTES + comment == tail.len() {
                    end = Some(position);
                    break;
                }
            }
            if position == 0 {
                break;
            }
            position -= 1;
        }
        let end = end.ok_or(XlsxError::Truncated)?;
        if end >= 20 && u32_at(&tail, end - 20) == ZIP64_LOCATOR_SIGNATURE {
            return Err(XlsxError::Zip64Unsupported);
        }
        let disk = u16_at(&tail, end + 4);
        let directory_disk = u16_at(&tail, end + 6);
        let disk_entries = u16_at(&tail, end + 8);
        let total_entries = u16_at(&tail, end + 10);
        let directory_size = u32_at(&tail, end + 12);
        let directory_offset = u32_at(&tail, end + 16);
        if total_entries == 0xffff || directory_size == u32::MAX || directory_offset == u32::MAX {
            return Err(XlsxError::Zip64Unsupported);
        }
        if disk != 0 || directory_disk != 0 || disk_entries != total_entries {
            return Err(XlsxError::MalformedZip(
                "multi-disk archives are not supported",
            ));
        }
        let entry_count = usize::from(total_entries);
        if entry_count > limits.max_zip_entries {
            return Err(XlsxError::TooManyEntries {
                limit: limits.max_zip_entries,
            });
        }
        let end_offset = tail_start + end as u64;
        let directory_end = u64::from(directory_offset) + u64::from(directory_size);
        if directory_end > end_offset {
            return Err(XlsxError::Truncated);
        }
        let directory_length = usize::try_from(directory_size).unwrap_or(usize::MAX);
        if directory_length > limits.max_central_directory_bytes {
            return Err(XlsxError::MalformedZip(
                "central directory exceeds its limit",
            ));
        }
        let mut directory = zeroed(directory_length)?;
        read_exact(source, u64::from(directory_offset), &mut directory)?;

        let mut entries: Vec<ZipEntry> = Vec::new();
        entries
            .try_reserve_exact(entry_count)
            .map_err(|_| XlsxError::AllocationFailed)?;
        let mut cursor = 0_usize;
        for _ in 0..entry_count {
            let header = directory.get(cursor..cursor + CENTRAL_HEADER_BYTES).ok_or(
                XlsxError::MalformedZip("central directory record is cut short"),
            )?;
            if u32_at(header, 0) != CENTRAL_HEADER_SIGNATURE {
                return Err(XlsxError::MalformedZip("bad central directory signature"));
            }
            let flags = u16_at(header, 8);
            let method = u16_at(header, 10);
            let crc32 = u32_at(header, 16);
            let compressed = u32_at(header, 20);
            let uncompressed = u32_at(header, 24);
            let name_length = usize::from(u16_at(header, 28));
            let extra_length = usize::from(u16_at(header, 30));
            let comment_length = usize::from(u16_at(header, 32));
            let local_header = u32_at(header, 42);
            let name_start = cursor + CENTRAL_HEADER_BYTES;
            let name_bytes = directory
                .get(name_start..name_start + name_length)
                .ok_or(XlsxError::MalformedZip("entry name is cut short"))?;
            if name_length > MAX_NAME_BYTES {
                return Err(XlsxError::MalformedZip("entry name is too long"));
            }
            let name = std::str::from_utf8(name_bytes)
                .map_err(|_| XlsxError::MalformedZip("entry name is not UTF-8"))?;
            if compressed == u32::MAX || uncompressed == u32::MAX || local_header == u32::MAX {
                return Err(XlsxError::Zip64Unsupported);
            }
            if flags & 1 != 0 {
                return Err(XlsxError::EncryptedEntry {
                    name: name.to_owned(),
                });
            }
            if entries
                .iter()
                .any(|entry| entry.name.eq_ignore_ascii_case(name))
            {
                return Err(XlsxError::DuplicateEntry {
                    name: name.to_owned(),
                });
            }
            entries.push(ZipEntry {
                name: name.into(),
                method,
                crc32,
                compressed: u64::from(compressed),
                uncompressed: u64::from(uncompressed),
                local_header: u64::from(local_header),
            });
            cursor = name_start + name_length + extra_length + comment_length;
        }
        Ok(Self {
            entries,
            central_directory: u64::from(directory_offset),
        })
    }

    /// Finds an entry by part name. OPC part names are case-insensitive.
    pub(crate) fn find(&self, name: &str) -> Option<&ZipEntry> {
        self.entries
            .iter()
            .find(|entry| entry.name.eq_ignore_ascii_case(name))
    }

    /// Opens one entry for streaming after checking the declared sizes.
    pub(crate) fn open<R: PositionedRead + ?Sized>(
        &self,
        source: &R,
        entry: &ZipEntry,
        limits: &XlsxLimits,
    ) -> Result<EntryStream, XlsxError> {
        if entry.method != METHOD_STORED && entry.method != METHOD_DEFLATE {
            return Err(XlsxError::UnsupportedCompression {
                name: entry.name.to_string(),
                method: entry.method,
            });
        }
        if entry.uncompressed > limits.max_entry_bytes {
            return Err(XlsxError::EntryTooLarge {
                name: entry.name.to_string(),
                limit: limits.max_entry_bytes,
            });
        }
        if ratio_exceeded(entry.uncompressed, entry.compressed, limits) {
            return Err(XlsxError::CompressionRatio {
                name: entry.name.to_string(),
                limit: limits.max_compression_ratio,
            });
        }
        if entry.method == METHOD_STORED && entry.compressed != entry.uncompressed {
            return Err(XlsxError::MalformedZip("stored entry sizes differ"));
        }
        let mut header = [0_u8; LOCAL_HEADER_BYTES];
        read_exact(source, entry.local_header, &mut header)?;
        if u32_at(&header, 0) != LOCAL_HEADER_SIGNATURE {
            return Err(XlsxError::MalformedZip("bad local header signature"));
        }
        let name_length = u16_at(&header, 26);
        let extra_length = u16_at(&header, 28);
        if usize::from(name_length) != entry.name.len() {
            return Err(XlsxError::MalformedZip(
                "local and central entry names differ",
            ));
        }
        let mut name = [0_u8; MAX_NAME_BYTES];
        let name = &mut name[..usize::from(name_length)];
        read_exact(source, entry.local_header + LOCAL_HEADER_BYTES as u64, name)?;
        if !name.eq_ignore_ascii_case(entry.name.as_bytes()) {
            return Err(XlsxError::MalformedZip(
                "local and central entry names differ",
            ));
        }
        let data = entry.local_header
            + LOCAL_HEADER_BYTES as u64
            + u64::from(name_length)
            + u64::from(extra_length);
        if data + entry.compressed > self.central_directory {
            return Err(XlsxError::Truncated);
        }
        EntryStream::new(entry, data, limits)
    }
}

fn ratio_exceeded(produced: u64, consumed: u64, limits: &XlsxLimits) -> bool {
    produced > limits.ratio_check_after_bytes
        && produced > consumed.saturating_mul(limits.max_compression_ratio)
}

/// Incremental CRC-32 (IEEE), as stored in ZIP headers.
#[derive(Clone, Copy, Debug)]
struct Crc32(u32);

const CRC_TABLE: [u32; 256] = {
    let mut table = [0_u32; 256];
    let mut index = 0;
    while index < 256 {
        #[allow(clippy::cast_possible_truncation, reason = "index is below 256")]
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 == 1 {
                0xedb8_8320 ^ (value >> 1)
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
};

impl Crc32 {
    const fn new() -> Self {
        Self(u32::MAX)
    }

    fn update(&mut self, bytes: &[u8]) {
        let mut value = self.0;
        for byte in bytes {
            value = CRC_TABLE[((value ^ u32::from(*byte)) & 0xff) as usize] ^ (value >> 8);
        }
        self.0 = value;
    }

    const fn finish(self) -> u32 {
        !self.0
    }
}

/// A streamed, bounded view of one entry's uncompressed bytes.
///
/// Decompressed bytes land in a 32 KiB ring that is also the deflate window.
/// `available` returns the bytes produced but not yet consumed. The stream
/// checks the produced size against the declared size and the compression
/// ratio limit as it goes, so lying headers cannot make it read further.
pub(crate) struct EntryStream {
    name: Box<str>,
    method: u16,
    data_offset: u64,
    compressed: u64,
    declared: u64,
    expected_crc: u32,
    max_entry_bytes: u64,
    max_ratio: u64,
    ratio_after: u64,
    inflater: Box<DecompressorOxide>,
    window: Box<[u8]>,
    read_position: usize,
    write_end: usize,
    input: Vec<u8>,
    input_position: usize,
    input_length: usize,
    consumed: u64,
    produced: u64,
    crc: Crc32,
    done: bool,
}

impl EntryStream {
    fn new(entry: &ZipEntry, data_offset: u64, limits: &XlsxLimits) -> Result<Self, XlsxError> {
        let window = zeroed(WINDOW_BYTES)?.into_boxed_slice();
        Ok(Self {
            name: entry.name.clone(),
            method: entry.method,
            data_offset,
            compressed: entry.compressed,
            declared: entry.uncompressed,
            expected_crc: entry.crc32,
            max_entry_bytes: limits.max_entry_bytes,
            max_ratio: limits.max_compression_ratio,
            ratio_after: limits.ratio_check_after_bytes,
            inflater: Box::default(),
            window,
            read_position: 0,
            write_end: 0,
            input: Vec::new(),
            input_position: 0,
            input_length: 0,
            consumed: 0,
            produced: 0,
            crc: Crc32::new(),
            done: false,
        })
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// Compressed bytes consumed so far.
    pub(crate) const fn consumed(&self) -> u64 {
        self.consumed
    }

    /// Uncompressed bytes produced so far.
    pub(crate) const fn produced(&self) -> u64 {
        self.produced
    }

    /// Bytes that are decompressed but not yet consumed.
    pub(crate) fn available(&self) -> &[u8] {
        &self.window[self.read_position..self.write_end]
    }

    pub(crate) fn consume(&mut self, count: usize) {
        self.read_position = (self.read_position + count).min(self.write_end);
    }

    /// A copy that keeps the decompressor state and window but not the input
    /// buffer, which is refilled from the file when the copy resumes.
    pub(crate) fn checkpoint(&self) -> Self {
        Self {
            name: self.name.clone(),
            method: self.method,
            data_offset: self.data_offset,
            compressed: self.compressed,
            declared: self.declared,
            expected_crc: self.expected_crc,
            max_entry_bytes: self.max_entry_bytes,
            max_ratio: self.max_ratio,
            ratio_after: self.ratio_after,
            inflater: self.inflater.clone(),
            window: self.window.clone(),
            read_position: self.read_position,
            write_end: self.write_end,
            input: Vec::new(),
            input_position: 0,
            input_length: 0,
            consumed: self.consumed,
            produced: self.produced,
            crc: self.crc,
            done: self.done,
        }
    }

    /// Bytes retained by this stream, for memory accounting.
    pub(crate) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<DecompressorOxide>() + self.window.len() + self.input.capacity()
    }

    /// Produces more bytes once the available ones are consumed. Returns
    /// `false` at the end of the entry.
    pub(crate) fn fill<R: PositionedRead + ?Sized>(
        &mut self,
        source: &R,
    ) -> Result<bool, XlsxError> {
        if self.read_position < self.write_end {
            return Ok(true);
        }
        if self.done {
            return Ok(false);
        }
        let out_position = self.write_end % WINDOW_BYTES;
        if self.method == METHOD_STORED {
            let remaining = self.compressed - self.consumed;
            let count = usize::try_from(remaining)
                .unwrap_or(usize::MAX)
                .min(WINDOW_BYTES - out_position);
            if count == 0 {
                return self.finish().map(|()| false);
            }
            read_exact(
                source,
                self.data_offset + self.consumed,
                &mut self.window[out_position..out_position + count],
            )?;
            self.consumed += count as u64;
            self.accept_output(out_position, count)?;
            if self.consumed == self.compressed {
                self.finish()?;
            }
            return Ok(true);
        }
        loop {
            if self.input_position == self.input_length {
                self.refill_input(source)?;
            }
            let more_input = self.consumed + ((self.input_length - self.input_position) as u64)
                < self.compressed;
            let flags = if more_input {
                inflate_flags::TINFL_FLAG_HAS_MORE_INPUT
            } else {
                0
            };
            let (status, read, written) = decompress(
                &mut self.inflater,
                &self.input[self.input_position..self.input_length],
                &mut self.window,
                out_position,
                flags,
            );
            self.input_position += read;
            self.consumed += read as u64;
            if written > 0 {
                self.accept_output(out_position, written)?;
            }
            match status {
                TINFLStatus::Done => {
                    self.finish()?;
                    return Ok(written > 0);
                }
                TINFLStatus::HasMoreOutput => {
                    if written > 0 {
                        return Ok(true);
                    }
                    return Err(self.corrupt());
                }
                TINFLStatus::NeedsMoreInput => {
                    if written > 0 {
                        return Ok(true);
                    }
                    if self.input_position < self.input_length && read == 0 {
                        return Err(self.corrupt());
                    }
                    if !more_input && self.input_position == self.input_length {
                        return Err(XlsxError::Truncated);
                    }
                }
                _ => return Err(self.corrupt()),
            }
        }
    }

    fn refill_input<R: PositionedRead + ?Sized>(&mut self, source: &R) -> Result<(), XlsxError> {
        if self.input.is_empty() {
            self.input = zeroed(INPUT_BYTES)?;
        }
        let remaining = self.compressed - self.consumed;
        let count = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(self.input.len());
        read_exact(
            source,
            self.data_offset + self.consumed,
            &mut self.input[..count],
        )?;
        self.input_position = 0;
        self.input_length = count;
        Ok(())
    }

    fn accept_output(&mut self, start: usize, count: usize) -> Result<(), XlsxError> {
        self.crc.update(&self.window[start..start + count]);
        self.read_position = start;
        self.write_end = start + count;
        self.produced += count as u64;
        if self.produced > self.declared || self.produced > self.max_entry_bytes {
            return Err(XlsxError::SizeMismatch {
                name: self.name.to_string(),
            });
        }
        let limits = XlsxLimits {
            max_compression_ratio: self.max_ratio,
            ratio_check_after_bytes: self.ratio_after,
            ..XlsxLimits::standard()
        };
        if ratio_exceeded(self.produced, self.consumed, &limits) {
            return Err(XlsxError::CompressionRatio {
                name: self.name.to_string(),
                limit: self.max_ratio,
            });
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), XlsxError> {
        self.done = true;
        if self.produced != self.declared {
            return Err(XlsxError::SizeMismatch {
                name: self.name.to_string(),
            });
        }
        if self.crc.finish() != self.expected_crc {
            return Err(XlsxError::ChecksumMismatch {
                name: self.name.to_string(),
            });
        }
        Ok(())
    }

    fn corrupt(&self) -> XlsxError {
        XlsxError::CorruptData {
            name: self.name.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Crc32;

    #[test]
    fn crc32_matches_the_published_check_value() {
        let mut crc = Crc32::new();
        crc.update(b"123456789");
        assert_eq!(crc.finish(), 0xcbf4_3926);
    }
}
