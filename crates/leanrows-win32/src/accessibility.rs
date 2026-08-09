//! Safe, bounded accessible-name composition for cached rows.

use std::char::decode_utf16;

/// Maximum UTF-16 storage for one accessible name, including its terminal NUL.
pub const MAX_ACCESSIBLE_NAME_UTF16_UNITS: usize = 512;

/// Immutable, NUL-terminated UTF-16 text used by the native annotation bridge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessibleName {
    utf16: Box<[u16]>,
}

impl AccessibleName {
    /// Composes a concise row name from semantic field labels and values.
    #[must_use]
    pub fn compose(absolute_row: u64, fields: &[(&str, &str)]) -> Self {
        let mut builder = NameBuilder::new();
        builder.push_str("Row ");
        builder.push_str(&absolute_row.saturating_add(1).to_string());
        for (label, value) in fields {
            if label.is_empty() || value.is_empty() {
                continue;
            }
            builder.push_str("; ");
            builder.push_str(label);
            builder.push_str(": ");
            builder.push_str(value);
        }
        builder.finish()
    }

    pub(crate) fn from_utf16_cells<'a>(
        absolute_row: u64,
        cells: impl IntoIterator<Item = &'a [u16]>,
    ) -> Self {
        let mut builder = NameBuilder::new();
        builder.push_str("Row ");
        builder.push_str(&absolute_row.saturating_add(1).to_string());
        for (index, cell) in cells.into_iter().enumerate() {
            builder.push_str("; Column ");
            builder.push_str(&index.saturating_add(1).to_string());
            builder.push_str(": ");
            if cell
                .iter()
                .copied()
                .take_while(|unit| *unit != 0)
                .next()
                .is_none()
            {
                builder.push_str("(empty)");
            } else {
                builder.push_utf16(cell.iter().copied().take_while(|unit| *unit != 0));
            }
        }
        builder.finish()
    }

    pub(crate) fn from_preview_utf16(absolute_row: u64, cell: &[u16]) -> Self {
        let mut builder = NameBuilder::new();
        builder.push_str("Row ");
        builder.push_str(&absolute_row.saturating_add(1).to_string());
        builder.push_str("; Preview: ");
        if cell
            .iter()
            .copied()
            .take_while(|unit| *unit != 0)
            .next()
            .is_none()
        {
            builder.push_str("(empty)");
        } else {
            builder.push_utf16(cell.iter().copied().take_while(|unit| *unit != 0));
        }
        builder.finish()
    }

    /// Returns persistent UTF-16 storage including the terminal NUL.
    #[must_use]
    pub fn as_utf16(&self) -> &[u16] {
        &self.utf16
    }
}

struct NameBuilder {
    units: Vec<u16>,
    truncated: bool,
    last_was_space: bool,
}

impl NameBuilder {
    fn new() -> Self {
        Self {
            units: Vec::with_capacity(MAX_ACCESSIBLE_NAME_UTF16_UNITS),
            truncated: false,
            last_was_space: false,
        }
    }

    fn push_str(&mut self, text: &str) {
        for character in text.chars() {
            self.push_char(character);
        }
    }

    fn push_utf16(&mut self, units: impl IntoIterator<Item = u16>) {
        for decoded in decode_utf16(units) {
            self.push_char(decoded.unwrap_or(char::REPLACEMENT_CHARACTER));
        }
    }

    fn push_char(&mut self, character: char) {
        if self.truncated {
            return;
        }
        let normalized = if character.is_control() || character.is_whitespace() {
            ' '
        } else {
            character
        };
        if normalized == ' ' && self.last_was_space {
            return;
        }
        let mut encoded = [0_u16; 2];
        let encoded = normalized.encode_utf16(&mut encoded);
        if self.units.len().saturating_add(encoded.len()) >= MAX_ACCESSIBLE_NAME_UTF16_UNITS {
            self.truncated = true;
            return;
        }
        self.units.extend_from_slice(encoded);
        self.last_was_space = normalized == ' ';
    }

    fn finish(mut self) -> AccessibleName {
        if self.truncated {
            while self.units.len().saturating_add(2) > MAX_ACCESSIBLE_NAME_UTF16_UNITS {
                pop_utf16_character(&mut self.units);
            }
            self.units.push(0x2026);
        }
        self.units.push(0);
        AccessibleName {
            utf16: self.units.into_boxed_slice(),
        }
    }
}

fn pop_utf16_character(units: &mut Vec<u16>) {
    let Some(last) = units.pop() else {
        return;
    };
    if (0xDC00..=0xDFFF).contains(&last)
        && units
            .last()
            .is_some_and(|unit| (0xD800..=0xDBFF).contains(unit))
    {
        units.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::{AccessibleName, MAX_ACCESSIBLE_NAME_UTF16_UNITS};

    fn decoded(name: &AccessibleName) -> String {
        let text = name.as_utf16();
        String::from_utf16_lossy(&text[..text.len().saturating_sub(1)])
    }

    #[test]
    fn semantic_name_is_one_based_and_whitespace_is_compact() {
        let name = AccessibleName::compose(
            41,
            &[("Timestamp", "10:15:02"), ("Message", "ready\r\nnow")],
        );
        assert_eq!(
            decoded(&name),
            "Row 42; Timestamp: 10:15:02; Message: ready now"
        );
        assert_eq!(name.as_utf16().last(), Some(&0));
    }

    #[test]
    fn cap_preserves_utf16_boundaries_and_adds_ellipsis() {
        let long = "😀".repeat(MAX_ACCESSIBLE_NAME_UTF16_UNITS);
        let name = AccessibleName::compose(u64::MAX, &[("Preview", &long)]);
        let text = decoded(&name);
        assert!(name.as_utf16().len() <= MAX_ACCESSIBLE_NAME_UTF16_UNITS);
        assert!(text.ends_with('…'));
        assert!(!text.contains(char::REPLACEMENT_CHARACTER));
        assert!(text.starts_with("Row 18446744073709551615; Preview: "));
    }

    #[test]
    fn empty_fields_are_omitted() {
        let name = AccessibleName::compose(0, &[("", "ignored"), ("Value", "")]);
        assert_eq!(decoded(&name), "Row 1");
    }

    #[test]
    fn utf16_cells_replace_malformed_sequences() {
        let malformed = [0xD800, u16::from(b'A'), 0];
        let name = AccessibleName::from_utf16_cells(4, [&malformed[..]]);
        assert_eq!(decoded(&name), "Row 5; Column 1: �A");
    }

    #[test]
    fn semantic_cached_names_preserve_empty_fields_and_preview_labels() {
        let empty = [0_u16];
        let value: Vec<u16> = "value\0".encode_utf16().collect();
        let fields = AccessibleName::from_utf16_cells(0, [&empty[..], &value[..]]);
        assert_eq!(
            decoded(&fields),
            "Row 1; Column 1: (empty); Column 2: value"
        );

        let preview = AccessibleName::from_preview_utf16(2, &value);
        assert_eq!(decoded(&preview), "Row 3; Preview: value");
    }
}
