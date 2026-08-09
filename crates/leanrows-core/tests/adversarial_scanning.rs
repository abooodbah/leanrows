use leanrows_core::{
    AdaptiveRowIndex, CancellationToken, CsvBoundaryScanner, CsvDiagnosticKind, CsvDialect,
    DocumentFormat, EmitControl, EngineLimits, FieldProjectionLimits, FieldUtf8Status,
    LineBoundaryScanner, ManagedMemoryBudget, PositionedRead, RecordSpan, ScanError, ScanSession,
    ScanSessionConfig, ScanStatus, ViewportFormat, project_delimited_record, read_viewport,
};
use std::io;
use std::num::NonZeroUsize;

#[derive(Clone, Debug)]
struct MemorySource(Vec<u8>);

impl PositionedRead for MemorySource {
    fn size(&self) -> io::Result<u64> {
        u64::try_from(self.0.len()).map_err(io::Error::other)
    }

    fn read_at(&self, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
        let start = usize::try_from(offset).map_err(io::Error::other)?;
        if start >= self.0.len() {
            return Ok(0);
        }
        let count = destination.len().min(self.0.len() - start);
        destination[..count].copy_from_slice(&self.0[start..start + count]);
        Ok(count)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReferenceCsv {
    spans: Vec<RecordSpan>,
    unterminated: Option<(u64, u64)>,
}

fn reference_csv(input: &[u8]) -> Result<ReferenceCsv, Box<dyn std::error::Error>> {
    let mut spans = Vec::new();
    let mut record_start = 0_usize;
    let mut cursor = 0_usize;
    let mut field_start = true;
    let mut quoted = false;

    while cursor < input.len() {
        let byte = input[cursor];
        if quoted {
            if byte == b'"' {
                if input.get(cursor + 1) == Some(&b'"') {
                    cursor += 2;
                } else {
                    quoted = false;
                    field_start = false;
                    cursor += 1;
                }
            } else {
                cursor += 1;
            }
            continue;
        }

        if byte == b'\r' || byte == b'\n' {
            spans.push(RecordSpan::new(
                u64::try_from(record_start)?,
                u64::try_from(cursor)?,
            )?);
            cursor += if byte == b'\r' && input.get(cursor + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
            record_start = cursor;
            field_start = true;
        } else if field_start && byte == b'"' {
            quoted = true;
            field_start = false;
            cursor += 1;
        } else {
            field_start = byte == b',';
            cursor += 1;
        }
    }

    if record_start < input.len() {
        spans.push(RecordSpan::new(
            u64::try_from(record_start)?,
            u64::try_from(input.len())?,
        )?);
    }
    let unterminated = if quoted {
        Some((u64::try_from(input.len())?, u64::try_from(record_start)?))
    } else {
        None
    };
    Ok(ReferenceCsv {
        spans,
        unterminated,
    })
}

fn scan_csv(input: &[u8], chunk_size: usize) -> Result<ReferenceCsv, ScanError> {
    let mut scanner = CsvBoundaryScanner::new(CsvDialect::csv());
    let mut spans = Vec::new();
    for chunk in input.chunks(chunk_size) {
        let progress = scanner.feed(chunk, |span| {
            spans.push(span);
            EmitControl::Continue
        })?;
        assert_eq!(progress.status(), ScanStatus::Complete);
        assert_eq!(progress.consumed(), chunk.len());
    }
    let finish = scanner.finish(|span| {
        spans.push(span);
        EmitControl::Continue
    });
    let unterminated = finish.diagnostic().map(|diagnostic| {
        assert_eq!(
            diagnostic.kind(),
            CsvDiagnosticKind::UnterminatedQuotedField
        );
        (diagnostic.offset(), diagnostic.record_start())
    });
    Ok(ReferenceCsv {
        spans,
        unterminated,
    })
}

fn reference_lines(input: &[u8]) -> Result<Vec<RecordSpan>, Box<dyn std::error::Error>> {
    let mut spans = Vec::new();
    let mut start = 0_usize;
    let mut cursor = 0_usize;
    while cursor < input.len() {
        let byte = input[cursor];
        if byte == b'\r' || byte == b'\n' {
            spans.push(RecordSpan::new(
                u64::try_from(start)?,
                u64::try_from(cursor)?,
            )?);
            cursor += if byte == b'\r' && input.get(cursor + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
            start = cursor;
        } else {
            cursor += 1;
        }
    }
    if start < input.len() {
        spans.push(RecordSpan::new(
            u64::try_from(start)?,
            u64::try_from(input.len())?,
        )?);
    }
    Ok(spans)
}

fn scan_lines(input: &[u8], chunk_size: usize) -> Result<Vec<RecordSpan>, ScanError> {
    let mut scanner = LineBoundaryScanner::new();
    let mut spans = Vec::new();
    for chunk in input.chunks(chunk_size) {
        let progress = scanner.feed(chunk, |span| {
            spans.push(span);
            EmitControl::Continue
        })?;
        assert_eq!(progress.status(), ScanStatus::Complete);
        assert_eq!(progress.consumed(), chunk.len());
    }
    scanner.finish(|span| {
        spans.push(span);
        EmitControl::Continue
    });
    Ok(spans)
}

fn assert_partition(input: &[u8], spans: &[RecordSpan]) -> Result<(), Box<dyn std::error::Error>> {
    let mut next_start = 0_usize;
    for span in spans {
        let start = usize::try_from(span.start())?;
        let end = usize::try_from(span.end())?;
        assert_eq!(start, next_start);
        assert!(end >= start && end <= input.len());
        if end == input.len() {
            next_start = end;
        } else {
            assert!(input[end] == b'\r' || input[end] == b'\n');
            next_start =
                end + usize::from(input[end] == b'\r' && input.get(end + 1) == Some(&b'\n')) + 1;
        }
    }
    assert_eq!(next_start, input.len());
    Ok(())
}

fn next_random(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn deterministic_random_bytes_match_independent_oracles_for_every_chunk_matrix()
-> Result<(), Box<dyn std::error::Error>> {
    const ALPHABET: &[u8] = b"ab,\"\r\n\t\0\xff";
    let mut state = 0xa076_1d64_78bd_642f_u64;
    for case in 0..512_usize {
        let length = usize::try_from(next_random(&mut state) % 257)?;
        let mut input = Vec::with_capacity(length);
        for _ in 0..length {
            let index = usize::try_from(next_random(&mut state))? % ALPHABET.len();
            input.push(ALPHABET[index]);
        }

        let expected_csv = reference_csv(&input)?;
        let expected_lines = reference_lines(&input)?;
        assert_partition(&input, &expected_csv.spans)?;
        assert_partition(&input, &expected_lines)?;
        for chunk_size in [1, 2, 3, 7, 31, length.max(1)] {
            assert_eq!(
                scan_csv(&input, chunk_size)?,
                expected_csv,
                "CSV case {case}, chunk {chunk_size}"
            );
            assert_eq!(
                scan_lines(&input, chunk_size)?,
                expected_lines,
                "line case {case}, chunk {chunk_size}"
            );
        }
    }
    Ok(())
}

#[test]
fn empty_and_trailing_separator_semantics_are_explicit() -> Result<(), Box<dyn std::error::Error>> {
    let cases: &[&[u8]] = &[
        b"",
        b"\n",
        b"\r",
        b"\r\n",
        b"a\n",
        b"a\n\n",
        b"a\r\n\r\n",
        b"\r\nlast",
    ];
    for input in cases {
        let expected_csv = reference_csv(input)?;
        let expected_lines = reference_lines(input)?;
        for chunk_size in [1, 2, 3] {
            assert_eq!(scan_csv(input, chunk_size)?, expected_csv);
            assert_eq!(scan_lines(input, chunk_size)?, expected_lines);
        }
    }
    assert!(scan_lines(b"", 1)?.is_empty());
    assert_eq!(scan_lines(b"\n", 1)?, vec![RecordSpan::new(0, 0)?]);
    assert_eq!(
        scan_lines(b"a\n\n", 1)?,
        vec![RecordSpan::new(0, 1)?, RecordSpan::new(2, 2)?]
    );
    Ok(())
}

#[test]
fn cancellation_after_an_emit_resumes_at_the_exact_unconsumed_byte()
-> Result<(), Box<dyn std::error::Error>> {
    let input = b"first\r\n\"multi\nline\",x\rthird\nfourth";
    let expected = reference_csv(input)?;
    let token = CancellationToken::new();
    let generation = token.snapshot();
    let mut scanner = CsvBoundaryScanner::new(CsvDialect::csv());
    let mut spans = Vec::new();
    let mut cancelled_once = false;
    let progress =
        scanner.feed_cancellable(input, &token, generation, NonZeroUsize::MIN, |span| {
            spans.push(span);
            if !cancelled_once {
                cancelled_once = true;
                token.cancel();
            }
            EmitControl::Continue
        })?;
    assert_eq!(progress.status(), ScanStatus::Cancelled);
    assert!(progress.consumed() > 0 && progress.consumed() < input.len());

    let remaining = &input[progress.consumed()..];
    let resumed = scanner.feed_cancellable(
        remaining,
        &token,
        token.snapshot(),
        NonZeroUsize::MIN,
        |span| {
            spans.push(span);
            EmitControl::Continue
        },
    )?;
    assert_eq!(resumed.status(), ScanStatus::Complete);
    assert_eq!(resumed.consumed(), remaining.len());
    let finish = scanner.finish(|span| {
        spans.push(span);
        EmitControl::Continue
    });
    assert_eq!(
        finish.diagnostic().map(leanrows_core::CsvDiagnostic::kind),
        None
    );
    assert_eq!(spans, expected.spans);
    Ok(())
}

#[test]
fn session_results_are_invariant_across_read_buffer_and_quantum_matrices()
-> Result<(), Box<dyn std::error::Error>> {
    const ALPHABET: &[u8] = b"abc,\"\r\n\xff";
    let mut random_state = 0xe703_7ed1_a0b4_28db_u64;
    let mut input = Vec::with_capacity(1_024);
    for _ in 0..1_024 {
        let index = usize::try_from(next_random(&mut random_state))? % ALPHABET.len();
        input.push(ALPHABET[index]);
    }
    let expected = reference_csv(&input)?;
    let source = MemorySource(input);

    for read_buffer in [1, 2, 7, 31] {
        for quantum in [1, 2, 5, 29, 257] {
            let token = CancellationToken::new();
            let mut generation = token.snapshot();
            let config = ScanSessionConfig::new(17, 1, read_buffer, 3);
            let mut session = ScanSession::new(&source, DocumentFormat::Csv, config)?;
            let mut steps = 0_usize;
            while !session.is_complete() {
                if steps % 17 == 16 {
                    let stale = generation;
                    generation = token.cancel();
                    let cancelled = session.step(&source, quantum, &token, stale)?;
                    assert_eq!(cancelled.status(), leanrows_core::ScanStepStatus::Cancelled);
                    assert_eq!(cancelled.processed_bytes(), 0);
                }
                session.step(&source, quantum, &token, generation)?;
                steps += 1;
                assert!(steps <= source.0.len().saturating_add(2_048));
            }
            assert_eq!(session.record_count(), u64::try_from(expected.spans.len())?);
            assert_eq!(
                session
                    .diagnostic()
                    .map(|item| (item.offset(), item.record_start())),
                expected.unterminated
            );
            assert!(session.index().len() <= session.index().capacity_limit());
        }
    }
    Ok(())
}

fn viewport_limits(
    records: usize,
    payload_bytes: usize,
) -> Result<EngineLimits, Box<dyn std::error::Error>> {
    let read_bytes = 7;
    let scratch_bytes = 128 * 1024;
    let budget = ManagedMemoryBudget::new(
        read_bytes + payload_bytes + scratch_bytes,
        read_bytes,
        payload_bytes,
        0,
        scratch_bytes,
    )?;
    Ok(EngineLimits::new(
        read_bytes,
        records,
        payload_bytes,
        payload_bytes,
        3,
        budget,
    )?)
}

#[test]
fn streaming_boundaries_and_field_projection_do_not_drift() -> Result<(), Box<dyn std::error::Error>>
{
    let mut state = 0x8ebc_6af0_9c88_c6e3_u64;
    let alphabet = [b'a', b',', b'"', b'\r', b'\n', 0xff, 0, 0xc3, 0xa9];
    let mut input = Vec::new();
    let mut expected_rows = Vec::new();
    for row in 0..96_usize {
        let field_count = 1 + usize::try_from(next_random(&mut state) % 6)?;
        let mut fields = Vec::with_capacity(field_count);
        for field_index in 0..field_count {
            if field_index > 0 {
                input.push(b',');
            }
            input.push(b'"');
            let length = usize::try_from(next_random(&mut state) % 21)?;
            let mut decoded = Vec::with_capacity(length);
            for _ in 0..length {
                let index = usize::try_from(next_random(&mut state))? % alphabet.len();
                let byte = alphabet[index];
                decoded.push(byte);
                input.push(byte);
                if byte == b'"' {
                    input.push(b'"');
                }
            }
            input.push(b'"');
            fields.push(decoded);
        }
        expected_rows.push(fields);
        match row % 3 {
            0 => input.push(b'\n'),
            1 => input.extend_from_slice(b"\r\n"),
            _ => input.push(b'\r'),
        }
    }

    let source = MemorySource(input.clone());
    let index = AdaptiveRowIndex::new(2, 1)?;
    let token = CancellationToken::new();
    let viewport = read_viewport(
        &source,
        ViewportFormat::Delimited(CsvDialect::csv()),
        &index,
        0,
        viewport_limits(expected_rows.len(), input.len())?,
        &token,
        token.snapshot(),
    )?;
    assert_eq!(viewport.records().len(), expected_rows.len());
    let projection_limits = FieldProjectionLimits::new(8, 256, 2_048)?;
    for (record, expected_fields) in viewport.records().iter().zip(&expected_rows) {
        assert!(!record.is_truncated());
        let projection = project_delimited_record(record, CsvDialect::csv(), projection_limits)?;
        assert!(!projection.are_fields_truncated());
        assert_eq!(projection.fields().len(), expected_fields.len());
        for (field, expected) in projection.fields().iter().zip(expected_fields) {
            assert_eq!(field.decoded_bytes(), expected);
            assert_eq!(
                field.utf8_status(),
                if std::str::from_utf8(expected).is_ok() {
                    FieldUtf8Status::Valid
                } else {
                    FieldUtf8Status::Invalid
                }
            );
            assert!(field.raw_span().start() >= record.span().start());
            assert!(field.raw_span().end() <= record.span().end());
        }
    }
    Ok(())
}

#[test]
fn scanners_report_offset_overflow_without_wrapping() -> Result<(), Box<dyn std::error::Error>> {
    let mut line = LineBoundaryScanner::at_offset(u64::MAX);
    assert_eq!(
        line.feed(b"x", |_| EmitControl::Continue),
        Err(ScanError::OffsetOverflow)
    );

    let mut csv = CsvBoundaryScanner::at_offset(CsvDialect::csv(), u64::MAX);
    assert_eq!(
        csv.feed(b"x", |_| EmitControl::Continue),
        Err(ScanError::OffsetOverflow)
    );

    let mut tail = LineBoundaryScanner::at_offset(u64::MAX - 1);
    assert_eq!(tail.feed(b"x", |_| EmitControl::Continue)?.consumed(), 1);
    let mut spans = Vec::new();
    tail.finish(|span| {
        spans.push(span);
        EmitControl::Continue
    });
    assert_eq!(spans, vec![RecordSpan::new(u64::MAX - 1, u64::MAX)?]);
    Ok(())
}
