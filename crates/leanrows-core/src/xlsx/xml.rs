//! A small, non-validating, streaming XML tokenizer for `SpreadsheetML` parts.
//!
//! It reports start tags, end tags and text. Comments and processing
//! instructions are skipped, CDATA is text, and a DOCTYPE is an error, so no
//! entity can be declared and nothing external is ever resolved. Only the five
//! predefined entities and numeric character references are decoded. Text is
//! collected only when the caller asks for it, up to a fixed limit; the full
//! length is still counted.

use super::XlsxError;

/// One complete token. Its name, attributes and text stay readable on the
/// tokenizer until the next call to `feed`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum XmlEvent {
    /// A start tag; `empty` is true for `<name/>`.
    Start { empty: bool },
    /// An end tag.
    End,
    /// Text collected since the previous tag, emitted before the next tag.
    Text,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    /// The very beginning, where a byte order mark may appear.
    Start,
    Text,
    TextEntity,
    TagOpen,
    Tag {
        end: bool,
        quote: u8,
    },
    Bang,
    Comment {
        dashes: u8,
    },
    CData {
        brackets: u8,
    },
    Instruction {
        question: bool,
    },
}

const MAX_ENTITY_BYTES: usize = 12;
const MAX_BANG_BYTES: usize = 8;
const MAX_INSTRUCTION_BYTES: usize = 256;

/// The tokenizer state. It holds no reference to its input, so it can be
/// copied at a token boundary and resumed later.
#[allow(
    clippy::struct_excessive_bools,
    reason = "text-collection flags are independent and reset together"
)]
#[derive(Debug)]
pub(crate) struct XmlTokenizer {
    part: Box<str>,
    state: State,
    tag: Vec<u8>,
    max_tag_bytes: usize,
    entity: Vec<u8>,
    instruction: Vec<u8>,
    collect: bool,
    text: Vec<u8>,
    text_limit: usize,
    text_total: u64,
    text_truncated: bool,
    pending_cr: bool,
    have_text: bool,
}

impl XmlTokenizer {
    pub(crate) fn new(part: &str, max_tag_bytes: usize, text_limit: usize) -> Self {
        Self {
            part: part.into(),
            state: State::Start,
            tag: Vec::new(),
            max_tag_bytes,
            entity: Vec::new(),
            instruction: Vec::new(),
            collect: false,
            text: Vec::new(),
            text_limit,
            text_total: 0,
            text_truncated: false,
            pending_cr: false,
            have_text: false,
        }
    }

    /// A copy for a checkpoint taken between tokens.
    pub(crate) fn checkpoint(&self) -> Self {
        Self {
            part: self.part.clone(),
            state: self.state,
            tag: Vec::new(),
            max_tag_bytes: self.max_tag_bytes,
            entity: self.entity.clone(),
            instruction: Vec::new(),
            collect: false,
            text: Vec::new(),
            text_limit: self.text_limit,
            text_total: 0,
            text_truncated: false,
            pending_cr: false,
            have_text: false,
        }
    }

    pub(crate) fn part(&self) -> &str {
        &self.part
    }

    /// Starts or stops collecting text. Starting clears the text buffer.
    pub(crate) fn collect_text(&mut self, collect: bool) {
        self.collect = collect;
        self.text.clear();
        self.text_total = 0;
        self.text_truncated = false;
        self.have_text = false;
        self.pending_cr = false;
    }

    /// Text collected since `collect_text(true)`, up to the limit.
    pub(crate) fn text(&self) -> &[u8] {
        &self.text
    }

    /// Full decoded length of the collected text, including dropped bytes.
    pub(crate) const fn text_total(&self) -> u64 {
        self.text_total
    }

    pub(crate) const fn text_truncated(&self) -> bool {
        self.text_truncated
    }

    #[cfg(test)]
    pub(crate) fn clear_text(&mut self) {
        self.text.clear();
        self.text_total = 0;
        self.text_truncated = false;
        self.have_text = false;
    }

    fn malformed(&self, reason: &'static str) -> XlsxError {
        XlsxError::MalformedXml {
            part: self.part.to_string(),
            reason,
        }
    }

    /// Local name of the current tag (the part after any prefix).
    pub(crate) fn name(&self) -> &[u8] {
        let body = self.tag_body();
        let end = body
            .iter()
            .position(|byte| is_space(*byte) || *byte == b'/')
            .unwrap_or(body.len());
        local(&body[..end])
    }

    fn tag_body(&self) -> &[u8] {
        let body = self.tag.strip_prefix(b"/").unwrap_or(&self.tag);
        body.strip_suffix(b"/").unwrap_or(body)
    }

    /// Decodes the attribute with this local name into `out`. When `prefixed`
    /// is true only a prefixed attribute matches (as in `r:id`).
    pub(crate) fn attribute(
        &self,
        name: &[u8],
        prefixed: bool,
        out: &mut Vec<u8>,
    ) -> Result<bool, XlsxError> {
        let body = self.tag_body();
        let mut cursor = body
            .iter()
            .position(|byte| is_space(*byte))
            .unwrap_or(body.len());
        loop {
            while cursor < body.len() && is_space(body[cursor]) {
                cursor += 1;
            }
            if cursor >= body.len() {
                return Ok(false);
            }
            let name_start = cursor;
            while cursor < body.len() && body[cursor] != b'=' && !is_space(body[cursor]) {
                cursor += 1;
            }
            let qualified = &body[name_start..cursor];
            while cursor < body.len() && is_space(body[cursor]) {
                cursor += 1;
            }
            if body.get(cursor) != Some(&b'=') {
                return Err(self.malformed("attribute without a value"));
            }
            cursor += 1;
            while cursor < body.len() && is_space(body[cursor]) {
                cursor += 1;
            }
            let quote = *body
                .get(cursor)
                .ok_or_else(|| self.malformed("attribute without a value"))?;
            if quote != b'"' && quote != b'\'' {
                return Err(self.malformed("attribute value is not quoted"));
            }
            cursor += 1;
            let value_start = cursor;
            while cursor < body.len() && body[cursor] != quote {
                cursor += 1;
            }
            if cursor >= body.len() {
                return Err(self.malformed("attribute value is not closed"));
            }
            let value = &body[value_start..cursor];
            cursor += 1;
            let has_prefix = qualified.contains(&b':');
            if local(qualified) == name && has_prefix == prefixed {
                out.clear();
                decode_entities(value, out).map_err(|reason| self.malformed(reason))?;
                return Ok(true);
            }
        }
    }

    fn push_text(&mut self, byte: u8) {
        if !self.collect {
            return;
        }
        self.have_text = true;
        self.text_total += 1;
        if self.text.len() < self.text_limit {
            self.text.push(byte);
        } else {
            self.text_truncated = true;
        }
    }

    fn push_raw_text(&mut self, byte: u8) {
        // XML end-of-line handling: CRLF and a lone CR become LF.
        if self.pending_cr {
            self.pending_cr = false;
            self.push_text(b'\n');
            if byte == b'\n' {
                return;
            }
        }
        if byte == b'\r' {
            self.pending_cr = true;
        } else {
            self.push_text(byte);
        }
    }

    fn flush_cr(&mut self) {
        if self.pending_cr {
            self.pending_cr = false;
            self.push_text(b'\n');
        }
    }

    fn push_tag(&mut self, byte: u8) -> Result<(), XlsxError> {
        if self.tag.len() >= self.max_tag_bytes {
            return Err(self.malformed("a tag is longer than the limit"));
        }
        self.tag.push(byte);
        Ok(())
    }

    /// Consumes input until one token completes. Returns the number of bytes
    /// used and the token, if one completed.
    #[allow(
        clippy::too_many_lines,
        reason = "one byte-level state machine reads best as one match"
    )]
    pub(crate) fn feed(&mut self, input: &[u8]) -> Result<(usize, Option<XmlEvent>), XlsxError> {
        let mut index = 0;
        while index < input.len() {
            let byte = input[index];
            index += 1;
            match self.state {
                State::Start => {
                    if self.instruction.len() < 3 && (byte == 0xef || !self.instruction.is_empty())
                    {
                        self.instruction.push(byte);
                        if self.instruction.len() == 3 {
                            if self.instruction != [0xef, 0xbb, 0xbf] {
                                return Err(self.malformed("bad byte order mark"));
                            }
                            self.instruction.clear();
                            self.state = State::Text;
                        }
                        continue;
                    }
                    if byte == 0xfe || byte == 0xff || byte == 0 {
                        return Err(XlsxError::UnsupportedEncoding {
                            part: self.part.to_string(),
                        });
                    }
                    self.state = State::Text;
                    index -= 1;
                }
                State::Text => {
                    if byte == b'<' {
                        self.flush_cr();
                        self.state = State::TagOpen;
                        self.tag.clear();
                        if self.collect && self.have_text {
                            return Ok((index, Some(XmlEvent::Text)));
                        }
                    } else if byte == b'&' {
                        self.flush_cr();
                        self.entity.clear();
                        self.state = State::TextEntity;
                    } else {
                        self.push_raw_text(byte);
                    }
                }
                State::TextEntity => {
                    if byte == b';' {
                        let mut decoded = [0_u8; 4];
                        let length = decode_entity(&self.entity, &mut decoded)
                            .map_err(|reason| self.malformed(reason))?;
                        for value in &decoded[..length] {
                            self.push_text(*value);
                        }
                        self.state = State::Text;
                    } else if self.entity.len() >= MAX_ENTITY_BYTES {
                        return Err(self.malformed("an entity reference is too long"));
                    } else {
                        self.entity.push(byte);
                    }
                }
                State::TagOpen => match byte {
                    b'/' => {
                        self.tag.push(b'/');
                        self.state = State::Tag {
                            end: true,
                            quote: 0,
                        };
                    }
                    b'!' => {
                        self.instruction.clear();
                        self.state = State::Bang;
                    }
                    b'?' => {
                        self.instruction.clear();
                        self.state = State::Instruction { question: false };
                    }
                    _ => {
                        if is_space(byte) || byte == b'>' {
                            return Err(self.malformed("a tag has no name"));
                        }
                        self.push_tag(byte)?;
                        self.state = State::Tag {
                            end: false,
                            quote: 0,
                        };
                    }
                },
                State::Tag { end, quote } => {
                    if quote != 0 {
                        if byte == quote {
                            self.state = State::Tag { end, quote: 0 };
                        }
                        self.push_tag(byte)?;
                    } else if byte == b'>' {
                        self.state = State::Text;
                        if end {
                            return Ok((index, Some(XmlEvent::End)));
                        }
                        let empty = self.tag.last() == Some(&b'/');
                        return Ok((index, Some(XmlEvent::Start { empty })));
                    } else {
                        if byte == b'"' || byte == b'\'' {
                            self.state = State::Tag { end, quote: byte };
                        } else if byte == b'<' {
                            return Err(self.malformed("'<' inside a tag"));
                        }
                        self.push_tag(byte)?;
                    }
                }
                State::Bang => {
                    self.instruction.push(byte);
                    let seen = self.instruction.as_slice();
                    if seen == b"--" {
                        self.state = State::Comment { dashes: 0 };
                    } else if seen == b"[CDATA[" {
                        self.state = State::CData { brackets: 0 };
                    } else if seen.eq_ignore_ascii_case(b"DOCTYPE") {
                        return Err(XlsxError::Doctype {
                            part: self.part.to_string(),
                        });
                    } else if !(b"--".starts_with(seen)
                        || b"[CDATA[".starts_with(seen)
                        || b"DOCTYPE".len() >= seen.len()
                            && b"DOCTYPE"[..seen.len()].eq_ignore_ascii_case(seen))
                        || seen.len() > MAX_BANG_BYTES
                    {
                        return Err(self.malformed("unsupported markup declaration"));
                    }
                }
                State::Comment { dashes } => {
                    if byte == b'>' && dashes >= 2 {
                        self.state = State::Text;
                    } else if byte == b'-' {
                        self.state = State::Comment {
                            dashes: dashes.saturating_add(1),
                        };
                    } else {
                        self.state = State::Comment { dashes: 0 };
                    }
                }
                State::CData { brackets } => {
                    if byte == b'>' && brackets >= 2 {
                        self.state = State::Text;
                    } else if byte == b']' {
                        if brackets >= 2 {
                            self.push_raw_text(b']');
                        } else {
                            self.state = State::CData {
                                brackets: brackets + 1,
                            };
                        }
                    } else {
                        for _ in 0..brackets {
                            self.push_raw_text(b']');
                        }
                        self.push_raw_text(byte);
                        self.state = State::CData { brackets: 0 };
                    }
                }
                State::Instruction { question } => {
                    if question && byte == b'>' {
                        self.check_declaration()?;
                        self.state = State::Text;
                    } else {
                        if question {
                            self.push_instruction(b'?');
                        }
                        if byte == b'?' {
                            self.state = State::Instruction { question: true };
                        } else {
                            self.push_instruction(byte);
                            self.state = State::Instruction { question: false };
                        }
                    }
                }
            }
        }
        Ok((index, None))
    }

    fn push_instruction(&mut self, byte: u8) {
        if self.instruction.len() < MAX_INSTRUCTION_BYTES {
            self.instruction.push(byte);
        }
    }

    /// Refuses an XML declaration that names an encoding other than UTF-8.
    fn check_declaration(&self) -> Result<(), XlsxError> {
        let Some(rest) = self.instruction.strip_prefix(b"xml") else {
            return Ok(());
        };
        if !rest.first().is_some_and(|byte| is_space(*byte)) {
            return Ok(());
        }
        let Some(position) = find(rest, b"encoding") else {
            return Ok(());
        };
        let after = &rest[position + b"encoding".len()..];
        let value: Vec<u8> = after
            .iter()
            .copied()
            .skip_while(|byte| is_space(*byte) || *byte == b'=')
            .skip(1)
            .take_while(|byte| *byte != b'"' && *byte != b'\'')
            .collect();
        if value.eq_ignore_ascii_case(b"utf-8") || value.eq_ignore_ascii_case(b"utf8") {
            Ok(())
        } else {
            Err(XlsxError::UnsupportedEncoding {
                part: self.part.to_string(),
            })
        }
    }

    /// Checks that the input did not end inside a token.
    pub(crate) fn finish(&self) -> Result<(), XlsxError> {
        match self.state {
            State::Start | State::Text => Ok(()),
            _ => Err(self.malformed("the part ends inside markup")),
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

const fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

fn local(qualified: &[u8]) -> &[u8] {
    match qualified.iter().rposition(|byte| *byte == b':') {
        Some(position) => &qualified[position + 1..],
        None => qualified,
    }
}

/// Decodes one entity body (between `&` and `;`) into UTF-8.
fn decode_entity(body: &[u8], out: &mut [u8; 4]) -> Result<usize, &'static str> {
    let character = match body {
        b"lt" => '<',
        b"gt" => '>',
        b"amp" => '&',
        b"quot" => '"',
        b"apos" => '\'',
        _ => {
            let digits = body
                .strip_prefix(b"#")
                .ok_or("undeclared entity reference")?;
            let (digits, radix) = match digits.strip_prefix(b"x") {
                Some(hex) => (hex, 16),
                None => (digits, 10),
            };
            if digits.is_empty() {
                return Err("empty character reference");
            }
            let mut value: u32 = 0;
            for digit in digits {
                let digit = char::from(*digit)
                    .to_digit(radix)
                    .ok_or("bad character reference")?;
                value = value
                    .checked_mul(radix)
                    .and_then(|value| value.checked_add(digit))
                    .ok_or("bad character reference")?;
            }
            match char::from_u32(value) {
                Some(character) if value != 0 => character,
                _ => return Err("bad character reference"),
            }
        }
    };
    Ok(character.encode_utf8(out).len())
}

/// Decodes entities in an attribute value.
fn decode_entities(value: &[u8], out: &mut Vec<u8>) -> Result<(), &'static str> {
    let mut cursor = 0;
    while cursor < value.len() {
        if value[cursor] == b'&' {
            let end = value[cursor..]
                .iter()
                .position(|byte| *byte == b';')
                .ok_or("unterminated entity reference")?;
            let mut decoded = [0_u8; 4];
            let length = decode_entity(&value[cursor + 1..cursor + end], &mut decoded)?;
            out.extend_from_slice(&decoded[..length]);
            cursor += end + 1;
        } else {
            out.push(value[cursor]);
            cursor += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{XmlEvent, XmlTokenizer};
    use crate::xlsx::XlsxError;

    fn events(input: &[u8], chunk: usize) -> Result<Vec<String>, XlsxError> {
        let mut tokenizer = XmlTokenizer::new("test.xml", 1_024, 64);
        tokenizer.collect_text(true);
        let mut out = Vec::new();
        for piece in input.chunks(chunk) {
            let mut rest = piece;
            while !rest.is_empty() {
                let (used, event) = tokenizer.feed(rest)?;
                rest = &rest[used..];
                match event {
                    Some(XmlEvent::Start { empty }) => out.push(format!(
                        "<{}{}>",
                        String::from_utf8_lossy(tokenizer.name()),
                        if empty { "/" } else { "" }
                    )),
                    Some(XmlEvent::End) => {
                        out.push(format!("</{}>", String::from_utf8_lossy(tokenizer.name())));
                    }
                    Some(XmlEvent::Text) => {
                        out.push(String::from_utf8_lossy(tokenizer.text()).into_owned());
                        tokenizer.clear_text();
                    }
                    None => {}
                }
            }
        }
        tokenizer.finish()?;
        Ok(out)
    }

    #[test]
    fn tokens_are_the_same_for_every_chunk_size() -> Result<(), XlsxError> {
        let input = b"\xef\xbb\xbf<?xml version=\"1.0\" encoding=\"UTF-8\"?><!-- c --><x:a b='1>2'>t&amp;&#x41;&#66;\r\nz<![CDATA[<raw>]]><c/></x:a>";
        let expected = events(input, input.len())?;
        assert_eq!(
            expected,
            ["<a>", "t&AB\nz", "<raw>", "<c/>", "</a>"].map(String::from)
        );
        for chunk in 1..input.len() {
            assert_eq!(events(input, chunk)?, expected, "chunk size {chunk}");
        }
        Ok(())
    }

    #[test]
    fn doctype_and_undeclared_entities_are_refused() {
        assert!(matches!(
            events(b"<!DOCTYPE a [<!ENTITY x 'y'>]><a/>", 3),
            Err(XlsxError::Doctype { .. })
        ));
        assert!(matches!(
            events(b"<a>&x;</a>", 1),
            Err(XlsxError::MalformedXml { .. })
        ));
        assert!(matches!(
            events(b"<?xml version='1.0' encoding='UTF-16'?><a/>", 4),
            Err(XlsxError::UnsupportedEncoding { .. })
        ));
        assert!(matches!(
            events(b"<a", 1),
            Err(XlsxError::MalformedXml { .. })
        ));
    }
}
