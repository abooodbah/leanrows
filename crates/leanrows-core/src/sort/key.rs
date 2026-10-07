use super::{SortDirection, SortError, SortMode, SortRequest};
use crate::{PositionedRead, RecordSpan, ViewportFormat};
use std::cmp::Ordering;
pub(super) const PREFIX: usize = 32;

fn short_number_order(a: Descriptor, b: Descriptor) -> Ordering {
    let mut left = a.prefix[..usize::try_from(a.len).unwrap_or(PREFIX)]
        .iter()
        .copied()
        .take_while(|b| !matches!(b, b'e' | b'E'))
        .filter(u8::is_ascii_digit)
        .skip(usize::try_from(a.leading).unwrap_or(PREFIX));
    let mut right = b.prefix[..usize::try_from(b.len).unwrap_or(PREFIX)]
        .iter()
        .copied()
        .take_while(|b| !matches!(b, b'e' | b'E'))
        .filter(u8::is_ascii_digit)
        .skip(usize::try_from(b.leading).unwrap_or(PREFIX));
    loop {
        let a = left.next();
        let b = right.next();
        if a.is_none() && b.is_none() {
            return Ordering::Equal;
        }
        let order = a.unwrap_or(b'0').cmp(&b.unwrap_or(b'0'));
        if order != Ordering::Equal {
            return order;
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Descriptor {
    pub row: u64,
    pub start: u64,
    pub end: u64,
    pub key_start: u64,
    pub key_end: u64,
    pub quoted: bool,
    pub prefix: [u8; PREFIX],
    pub len: u64,
    pub group: u8,
    pub sign: i8,
    pub magnitude: i128,
    pub leading: u64,
    pub digits: u64,
}
#[derive(Clone, Copy)]
enum FieldState {
    Start,
    Unquoted,
    Quoted,
    Closed,
}
pub(super) struct Locator {
    descriptor: Descriptor,
    cursor: u64,
    field_start: u64,
    column: usize,
    target: usize,
    state: FieldState,
    delimiter: Option<u8>,
    quote: u8,
}
impl Locator {
    pub fn new(row: u64, span: RecordSpan, format: ViewportFormat, target: usize) -> Self {
        let (delimiter, quote) = match format {
            ViewportFormat::Delimited(d) => (Some(d.delimiter()), d.quote()),
            ViewportFormat::Lines => (None, b'"'),
        };
        Self {
            descriptor: Descriptor {
                row,
                start: span.start(),
                end: span.end(),
                ..Descriptor::default()
            },
            cursor: span.start(),
            field_start: span.start(),
            column: 0,
            target,
            state: FieldState::Start,
            delimiter,
            quote,
        }
    }
    pub fn advance<R: PositionedRead>(
        &mut self,
        source: &R,
        budget: &mut usize,
    ) -> Result<Option<Descriptor>, SortError> {
        let mut buf = [0; 4096];
        while self.cursor < self.descriptor.end && *budget > 0 {
            let request = (*budget)
                .min(buf.len())
                .min(usize::try_from(self.descriptor.end - self.cursor).unwrap_or(usize::MAX));
            let n = checked_read(source, self.cursor, &mut buf[..request])?;
            for byte in &buf[..n] {
                let at = self.cursor;
                self.cursor += 1;
                *budget -= 1;
                if self.delimiter.is_some() {
                    match self.state {
                        FieldState::Start if *byte == self.quote => {
                            self.state = FieldState::Quoted;
                            if self.column == self.target {
                                self.descriptor.quoted = true;
                            }
                        }
                        FieldState::Quoted if *byte == self.quote => {
                            self.state = FieldState::Closed;
                        }
                        FieldState::Quoted => {}
                        FieldState::Closed if *byte == self.quote => {
                            self.state = FieldState::Quoted;
                        }
                        _ if Some(*byte) == self.delimiter => {
                            if self.column == self.target {
                                self.descriptor.key_start = self.field_start;
                                self.descriptor.key_end = at;
                                return Ok(Some(self.descriptor));
                            }
                            self.column += 1;
                            self.field_start = self.cursor;
                            self.state = FieldState::Start;
                        }
                        _ => self.state = FieldState::Unquoted,
                    }
                }
            }
        }
        if self.cursor == self.descriptor.end {
            if self.column == self.target {
                self.descriptor.key_start = self.field_start;
                self.descriptor.key_end = self.cursor;
            } else {
                self.descriptor.key_start = self.cursor;
                self.descriptor.key_end = self.cursor;
            }
            return Ok(Some(self.descriptor));
        }
        Ok(None)
    }
}
pub(super) fn checked_read<R: PositionedRead>(
    source: &R,
    offset: u64,
    buf: &mut [u8],
) -> Result<usize, SortError> {
    let n = source.read_at(offset, buf)?;
    if n == 0 || n > buf.len() {
        return Err(SortError::SourceChanged);
    }
    Ok(n)
}
pub(super) enum ByteStep {
    Byte(u8),
    End,
    Yield,
}
pub(super) struct Cursor {
    offset: u64,
    end: u64,
    buf: [u8; 4096],
    pos: usize,
    count: usize,
    quoted: bool,
    closed: bool,
    first: bool,
    quote: u8,
}
impl Cursor {
    fn new(d: Descriptor, quote: u8) -> Self {
        Self {
            offset: d.key_start,
            end: d.key_end,
            buf: [0; 4096],
            pos: 0,
            count: 0,
            quoted: d.quoted,
            closed: false,
            first: true,
            quote,
        }
    }
    fn next<R: PositionedRead>(
        &mut self,
        source: &R,
        budget: &mut usize,
    ) -> Result<ByteStep, SortError> {
        while *budget > 0 {
            if self.pos == self.count {
                if self.offset == self.end {
                    return Ok(ByteStep::End);
                }
                let size = self
                    .buf
                    .len()
                    .min(usize::try_from(self.end - self.offset).unwrap_or(usize::MAX));
                self.count = checked_read(source, self.offset, &mut self.buf[..size])?;
                self.pos = 0;
            }
            let byte = self.buf[self.pos];
            self.pos += 1;
            self.offset += 1;
            *budget -= 1;
            if self.first {
                self.first = false;
                if self.quoted {
                    continue;
                }
            }
            if self.quoted {
                if self.closed {
                    self.closed = false;
                    if byte == self.quote {
                        return Ok(ByteStep::Byte(byte));
                    }
                    self.quoted = false;
                    return Ok(ByteStep::Byte(byte));
                }
                if byte == self.quote {
                    self.closed = true;
                    continue;
                }
            }
            return Ok(ByteStep::Byte(byte));
        }
        Ok(ByteStep::Yield)
    }
}
#[derive(Default)]
// Independent sign/exponent/parser flags are not user-facing configuration.
#[allow(clippy::struct_excessive_bools)]
struct Number {
    state: u8,
    negative: bool,
    exp_negative: bool,
    exp: u64,
    exp_digits: u64,
    before_dot: u64,
    digits: u64,
    leading: u64,
    significant: bool,
    invalid: bool,
    exp_overflow: bool,
}
impl Number {
    fn push(&mut self, b: u8) {
        if self.invalid {
            return;
        }
        if b.is_ascii_whitespace() {
            if matches!(self.state, 1 | 4 | 5) {
                self.invalid = true;
            }
            if self.state != 0 {
                self.state = 7;
            }
            return;
        }
        match self.state {
            0 => match b {
                b'+' | b'-' => {
                    self.negative = b == b'-';
                    self.state = 1;
                }
                b'.' => self.state = 3,
                b'0'..=b'9' => {
                    self.state = 2;
                    self.digit(b, true);
                }
                _ => self.invalid = true,
            },
            1 | 2 => match b {
                b'0'..=b'9' => {
                    self.state = 2;
                    self.digit(b, true);
                }
                b'.' => self.state = 3,
                b'e' | b'E' if self.digits > 0 => self.state = 4,
                _ => self.invalid = true,
            },
            3 => match b {
                b'0'..=b'9' => self.digit(b, false),
                b'e' | b'E' if self.digits > 0 => self.state = 4,
                _ => self.invalid = true,
            },
            4 => match b {
                b'+' | b'-' => {
                    self.exp_negative = b == b'-';
                    self.state = 5;
                }
                b'0'..=b'9' => {
                    self.state = 6;
                    self.exponent(b);
                }
                _ => self.invalid = true,
            },
            5 | 6 if b.is_ascii_digit() => {
                self.state = 6;
                self.exponent(b);
            }
            _ => self.invalid = true,
        }
    }
    fn digit(&mut self, b: u8, integer: bool) {
        self.digits += 1;
        if integer {
            self.before_dot += 1;
        }
        if !self.significant && b == b'0' {
            self.leading += 1;
        }
        if b != b'0' {
            self.significant = true;
        }
    }
    fn exponent(&mut self, b: u8) {
        let next = self
            .exp
            .checked_mul(10)
            .and_then(|n| n.checked_add(u64::from(b - b'0')))
            .filter(|n| i64::try_from(*n).is_ok());
        if let Some(value) = next {
            self.exp = value;
        } else {
            self.exp_overflow = true;
        }
        self.exp_digits += 1;
    }
    fn finish(&self, d: &mut Descriptor) -> Result<(), SortError> {
        d.group = if d.len == 0 || (self.state == 0 && !self.invalid) {
            2
        } else {
            1
        };
        if self.invalid
            || self.digits == 0
            || matches!(self.state, 1 | 4 | 5)
            || (self.exp_digits == 0 && self.state == 6)
        {
            return Ok(());
        }
        if self.exp_overflow {
            return Err(SortError::NumericRange);
        }
        d.group = 0;
        d.leading = self.leading;
        d.digits = self.digits;
        d.sign = if !self.significant {
            0
        } else if self.negative {
            -1
        } else {
            1
        };
        let exp = i128::from(self.exp) * if self.exp_negative { -1 } else { 1 };
        d.magnitude = i128::from(self.before_dot) - i128::from(self.leading) + exp;
        Ok(())
    }
}
pub(super) struct Builder {
    descriptor: Descriptor,
    cursor: Cursor,
    number: Number,
}
impl Builder {
    pub fn new(d: Descriptor, quote: u8) -> Self {
        Self {
            descriptor: d,
            cursor: Cursor::new(d, quote),
            number: Number::default(),
        }
    }
    pub fn advance<R: PositionedRead>(
        &mut self,
        source: &R,
        budget: &mut usize,
        mode: SortMode,
    ) -> Result<Option<Descriptor>, SortError> {
        loop {
            match self.cursor.next(source, budget)? {
                ByteStep::Byte(b) => {
                    if self.descriptor.len < PREFIX as u64 {
                        self.descriptor.prefix[usize::try_from(self.descriptor.len).unwrap_or(0)] =
                            b;
                    }
                    self.descriptor.len += 1;
                    if mode == SortMode::Numeric {
                        self.number.push(b);
                    }
                }
                ByteStep::End => {
                    if mode == SortMode::Numeric {
                        self.number.finish(&mut self.descriptor)?;
                    }
                    return Ok(Some(self.descriptor));
                }
                ByteStep::Yield => return Ok(None),
            }
        }
    }
}
struct Digits {
    cursor: Cursor,
    skip: u64,
    ended: bool,
}
impl Digits {
    fn new(d: Descriptor, quote: u8) -> Self {
        Self {
            cursor: Cursor::new(d, quote),
            skip: d.leading,
            ended: false,
        }
    }
    fn next<R: PositionedRead>(
        &mut self,
        source: &R,
        budget: &mut usize,
    ) -> Result<ByteStep, SortError> {
        if self.ended {
            return Ok(ByteStep::End);
        }
        loop {
            match self.cursor.next(source, budget)? {
                ByteStep::Byte(b'e' | b'E') | ByteStep::End => {
                    self.ended = true;
                    return Ok(ByteStep::End);
                }
                ByteStep::Byte(b) if b.is_ascii_digit() => {
                    if self.skip > 0 {
                        self.skip -= 1;
                    } else {
                        return Ok(ByteStep::Byte(b));
                    }
                }
                ByteStep::Yield => return Ok(ByteStep::Yield),
                ByteStep::Byte(_) => {}
            }
        }
    }
}
// None means not read, Some(None) means EOF; preserves one side across a yield.
#[allow(clippy::option_option)]
pub(super) struct Comparison {
    a: Descriptor,
    b: Descriptor,
    request: SortRequest,
    ca: Cursor,
    cb: Cursor,
    da: Digits,
    db: Digits,
    numeric: bool,
    first: bool,
    left: Option<Option<u8>>,
}
impl Comparison {
    pub fn new(a: Descriptor, b: Descriptor, request: SortRequest, quote: u8) -> Self {
        Self {
            a,
            b,
            request,
            ca: Cursor::new(a, quote),
            cb: Cursor::new(b, quote),
            da: Digits::new(a, quote),
            db: Digits::new(b, quote),
            numeric: request.mode == SortMode::Numeric && a.group == 0 && b.group == 0,
            first: true,
            left: None,
        }
    }
    fn finish(&self, value: Ordering) -> Ordering {
        let value = if self.request.direction == SortDirection::Descending {
            value.reverse()
        } else {
            value
        };
        value.then(self.a.row.cmp(&self.b.row))
    }
    pub fn advance<R: PositionedRead>(
        &mut self,
        source: &R,
        budget: &mut usize,
    ) -> Result<Option<Ordering>, SortError> {
        if self.first {
            self.first = false;
            if self.request.mode == SortMode::Numeric {
                let group = self.a.group.cmp(&self.b.group);
                if group != Ordering::Equal {
                    return Ok(Some(group));
                }
                if self.a.group == 2 {
                    return Ok(Some(self.a.row.cmp(&self.b.row)));
                }
            }
            if self.numeric {
                let sign = self.a.sign.cmp(&self.b.sign);
                if sign != Ordering::Equal {
                    return Ok(Some(self.finish(sign)));
                }
                if self.a.sign == 0 {
                    return Ok(Some(self.a.row.cmp(&self.b.row)));
                }
                let magnitude = self.a.magnitude.cmp(&self.b.magnitude);
                if magnitude != Ordering::Equal {
                    return Ok(Some(self.finish(if self.a.sign < 0 {
                        magnitude.reverse()
                    } else {
                        magnitude
                    })));
                }
                if self.a.len <= PREFIX as u64 && self.b.len <= PREFIX as u64 {
                    let value = short_number_order(self.a, self.b);
                    return Ok(Some(self.finish(if self.a.sign < 0 {
                        value.reverse()
                    } else {
                        value
                    })));
                }
            } else {
                let pa = &self.a.prefix
                    [..usize::try_from(self.a.len.min(PREFIX as u64)).unwrap_or(PREFIX)];
                let pb = &self.b.prefix
                    [..usize::try_from(self.b.len.min(PREFIX as u64)).unwrap_or(PREFIX)];
                let prefix = pa.cmp(pb);
                if prefix != Ordering::Equal {
                    return Ok(Some(self.finish(prefix)));
                }
                if self.a.len <= PREFIX as u64 && self.b.len <= PREFIX as u64 {
                    return Ok(Some(self.a.row.cmp(&self.b.row)));
                }
            }
        }
        loop {
            if self.left.is_none() {
                let value = if self.numeric {
                    self.da.next(source, budget)?
                } else {
                    self.ca.next(source, budget)?
                };
                match value {
                    ByteStep::Yield => return Ok(None),
                    ByteStep::Byte(b) => self.left = Some(Some(b)),
                    ByteStep::End => self.left = Some(None),
                }
            }
            let right = if self.numeric {
                self.db.next(source, budget)?
            } else {
                self.cb.next(source, budget)?
            };
            let right = match right {
                ByteStep::Yield => return Ok(None),
                ByteStep::Byte(b) => Some(b),
                ByteStep::End => None,
            };
            let left = self.left.take().flatten();
            if left.is_none() && right.is_none() {
                return Ok(Some(self.a.row.cmp(&self.b.row)));
            }
            let value = if self.numeric {
                left.unwrap_or(b'0').cmp(&right.unwrap_or(b'0'))
            } else {
                left.cmp(&right)
            };
            if value != Ordering::Equal {
                return Ok(Some(self.finish(if self.numeric && self.a.sign < 0 {
                    value.reverse()
                } else {
                    value
                })));
            }
        }
    }
}
