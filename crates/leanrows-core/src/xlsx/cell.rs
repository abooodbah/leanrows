//! Cell-text rules: number formats, date serials and booleans.
//!
//! docs/XLSX.md is the reference for these rules. The Android port follows
//! the same document, so a change here needs a change there.

use std::fmt::Write as _;

/// How a numeric cell is shown, decided by its number format.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NumberKind {
    /// The stored `<v>` text, unchanged.
    Plain,
    /// `YYYY-MM-DD`.
    Date,
    /// `YYYY-MM-DD HH:MM:SS`.
    DateTime,
    /// `HH:MM:SS`.
    Time,
}

/// Classifies a built-in number format id.
pub(crate) const fn builtin_kind(id: u32) -> NumberKind {
    match id {
        14..=17 => NumberKind::Date,
        18..=21 | 45..=47 => NumberKind::Time,
        22 => NumberKind::DateTime,
        _ => NumberKind::Plain,
    }
}

/// Classifies a custom `formatCode`.
///
/// Letters inside double quotes, after a backslash, after `_` or `*`, and
/// inside `[...]` are ignored, except that `[h]`, `[m]` and `[s]` (any
/// repeat) count as time. `h` or `s` is a time part. `d` or `y` is a date
/// part. `m` is a date part only when there is no time part.
pub(crate) fn format_code_kind(code: &[u8]) -> NumberKind {
    let mut date = false;
    let mut time = false;
    let mut month = false;
    let mut index = 0;
    while index < code.len() {
        let byte = code[index].to_ascii_lowercase();
        match byte {
            b'"' => {
                index += 1;
                while index < code.len() && code[index] != b'"' {
                    index += 1;
                }
            }
            b'\\' | b'_' | b'*' => index += 1,
            b'[' => {
                let start = index + 1;
                while index < code.len() && code[index] != b']' {
                    index += 1;
                }
                let inner = &code[start.min(index)..index];
                if !inner.is_empty()
                    && inner.iter().all(|letter| {
                        letter.eq_ignore_ascii_case(&inner[0])
                            && matches!(letter.to_ascii_lowercase(), b'h' | b'm' | b's')
                    })
                {
                    time = true;
                }
            }
            b'd' | b'y' => date = true,
            b'h' | b's' => time = true,
            b'm' => month = true,
            _ => {}
        }
        index += 1;
    }
    if month && !time {
        date = true;
    }
    match (date, time) {
        (true, true) => NumberKind::DateTime,
        (true, false) => NumberKind::Date,
        (false, true) => NumberKind::Time,
        (false, false) => NumberKind::Plain,
    }
}

const SECONDS_PER_DAY: i64 = 86_400;
/// The first serial that would fall after 9999-12-31 in the 1900 system.
const MAX_SERIAL: f64 = 2_958_466.0;

/// Days from 1970-01-01 to the given civil date (proleptic Gregorian).
const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The civil date of a day count from 1970-01-01.
const fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// Writes the date for a whole day count, or returns false if out of range.
fn write_date(out: &mut String, days: i64, date1904: bool) -> bool {
    let (year, month, day) = if date1904 {
        civil_from_days(days_from_civil(1904, 1, 1) + days)
    } else if days == 0 {
        (1900, 1, 0)
    } else if days == 60 {
        // Excel treats 1900 as a leap year and shows serial 60 as this date.
        (1900, 2, 29)
    } else if days < 60 {
        civil_from_days(days_from_civil(1899, 12, 31) + days)
    } else {
        civil_from_days(days_from_civil(1899, 12, 30) + days)
    };
    if !(1900..=9999).contains(&year) {
        return false;
    }
    let _ = write!(out, "{year:04}-{month:02}-{day:02}");
    true
}

fn write_time(out: &mut String, seconds: i64) {
    let _ = write!(
        out,
        "{:02}:{:02}:{:02}",
        seconds / 3_600,
        seconds / 60 % 60,
        seconds % 60
    );
}

/// Converts a stored numeric value to its display text. Returns `None` when
/// the stored text should be shown unchanged.
pub(crate) fn serial_text(stored: &[u8], kind: NumberKind, date1904: bool) -> Option<String> {
    if kind == NumberKind::Plain {
        return None;
    }
    let text = std::str::from_utf8(stored).ok()?.trim();
    let serial: f64 = text.parse().ok()?;
    if !serial.is_finite() || !(0.0..MAX_SERIAL).contains(&serial) {
        return None;
    }
    // Round to the nearest second first, then split into day and time.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the serial is checked to be in 0..2958466, so the product fits in i64"
    )]
    let total = (serial * 86_400.0).round() as i64;
    let days = total / SECONDS_PER_DAY;
    let seconds = total % SECONDS_PER_DAY;
    let mut out = String::new();
    match kind {
        NumberKind::Plain => return None,
        NumberKind::Date => {
            if !write_date(&mut out, days, date1904) {
                return None;
            }
        }
        NumberKind::DateTime => {
            if !write_date(&mut out, days, date1904) {
                return None;
            }
            out.push(' ');
            write_time(&mut out, seconds);
        }
        NumberKind::Time => write_time(&mut out, seconds),
    }
    Some(out)
}

/// The text of a `t="b"` cell.
pub(crate) fn boolean_text(stored: &[u8]) -> Option<&'static str> {
    match stored {
        b"1" => Some("TRUE"),
        b"0" => Some("FALSE"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{NumberKind, builtin_kind, format_code_kind, serial_text};

    #[test]
    fn format_codes_are_classified_outside_quotes_escapes_and_brackets() {
        let cases: [(&[u8], NumberKind); 14] = [
            (b"General", NumberKind::Plain),
            (b"0.00%", NumberKind::Plain),
            (b"\"Day \"0", NumberKind::Plain),
            (b"yyyy-mm-dd", NumberKind::Date),
            (b"yyyy\\-mm\\-dd", NumberKind::Date),
            (b"dd/mm/yyyy", NumberKind::Date),
            (b"mmm", NumberKind::Date),
            (b"h:mm:ss", NumberKind::Time),
            (b"mm:ss", NumberKind::Time),
            (b"[h]:mm:ss", NumberKind::Time),
            (b"[mm]", NumberKind::Time),
            (b"[Red][$-409]0.00", NumberKind::Plain),
            (b"yyyy-mm-dd h:mm:ss", NumberKind::DateTime),
            (b"_-* #,##0_-", NumberKind::Plain),
        ];
        for (code, expected) in cases {
            assert_eq!(
                format_code_kind(code),
                expected,
                "{}",
                String::from_utf8_lossy(code)
            );
        }
        assert_eq!(builtin_kind(14), NumberKind::Date);
        assert_eq!(builtin_kind(22), NumberKind::DateTime);
        assert_eq!(builtin_kind(46), NumberKind::Time);
        assert_eq!(builtin_kind(10), NumberKind::Plain);
        assert_eq!(builtin_kind(48), NumberKind::Plain);
    }

    #[test]
    fn serials_follow_the_excel_1900_and_1904_calendars() {
        let date = |text: &str| serial_text(text.as_bytes(), NumberKind::Date, false);
        assert_eq!(date("0").as_deref(), Some("1900-01-00"));
        assert_eq!(date("1").as_deref(), Some("1900-01-01"));
        assert_eq!(date("59").as_deref(), Some("1900-02-28"));
        assert_eq!(date("60").as_deref(), Some("1900-02-29"));
        assert_eq!(date("61").as_deref(), Some("1900-03-01"));
        assert_eq!(date("45351").as_deref(), Some("2024-02-29"));
        assert_eq!(date("2958465").as_deref(), Some("9999-12-31"));
        assert_eq!(date("2958466"), None);
        assert_eq!(date("-1"), None);
        assert_eq!(date("abc"), None);
        assert_eq!(
            serial_text(b"0", NumberKind::Date, true).as_deref(),
            Some("1904-01-01")
        );
        assert_eq!(
            serial_text(b"45351.999999", NumberKind::DateTime, false).as_deref(),
            Some("2024-03-01 00:00:00")
        );
        assert_eq!(
            serial_text(b"1.5", NumberKind::Time, false).as_deref(),
            Some("12:00:00")
        );
    }
}
