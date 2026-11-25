//! The interface in English, Simplified Chinese and Japanese, as in
//! `LeanMark`.
//!
//! Each language is one constant table with a field per visible string, so a
//! key missing from a language does not compile. The window shows one
//! language at a time: the reader's choice in the settings panel, or the
//! Windows display language when the choice is System. Messages that the
//! worker, the document engine and the operating system layers write in
//! English are put into the chosen language where the window shows them, by
//! matching them against the English table.

use std::sync::atomic::{AtomicU8, Ordering};

macro_rules! interface_strings {
    ($($key:ident),* $(,)?) => {
        /// One language's interface strings. `{name}` marks a value the
        /// code fills in, and `&` marks a menu or dialog access key.
        #[derive(Debug)]
        pub struct Strings {
            $(
                #[allow(missing_docs, reason = "each field is named after its string")]
                pub $key: &'static str,
            )*
        }

        impl Strings {
            /// Every key and its string, in table order.
            #[must_use]
            pub fn entries(&self) -> Vec<(&'static str, &'static str)> {
                vec![$((stringify!($key), self.$key)),*]
            }

            /// The string for `key`, if there is one.
            #[must_use]
            pub fn get(&self, key: &str) -> Option<&'static str> {
                match key {
                    $(stringify!($key) => Some(self.$key),)*
                    _ => None,
                }
            }
        }
    };
}

include!("localization/keys.rs");

mod chinese;
mod english;
mod japanese;

/// An interface language.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Language {
    /// English, the fallback.
    English,
    /// Simplified Chinese.
    ChineseSimplified,
    /// Japanese.
    Japanese,
}

impl Language {
    /// The interface languages, in the order the settings panel lists them.
    pub const ALL: [Self; 3] = [Self::English, Self::ChineseSimplified, Self::Japanese];

    /// The language tag: `en`, `zh-CN` or `ja`.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::ChineseSimplified => "zh-CN",
            Self::Japanese => "ja",
        }
    }

    /// The language's name in itself, as the settings panel shows it in
    /// every language.
    #[must_use]
    pub const fn native_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::ChineseSimplified => "简体中文",
            Self::Japanese => "日本語",
        }
    }

    const fn index(self) -> u8 {
        match self {
            Self::English => 0,
            Self::ChineseSimplified => 1,
            Self::Japanese => 2,
        }
    }

    const fn from_index(index: u8) -> Self {
        match index {
            1 => Self::ChineseSimplified,
            2 => Self::Japanese,
            _ => Self::English,
        }
    }
}

/// What the reader chose in the settings panel. Saved as `Language` under
/// `HKCU\Software\LeanRows`, beside `Theme`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LanguageSetting {
    /// Follow the Windows display language.
    #[default]
    System,
    /// English.
    English,
    /// Simplified Chinese.
    ChineseSimplified,
    /// Japanese.
    Japanese,
}

impl LanguageSetting {
    /// The choices, in the order the settings panel lists them.
    pub const ALL: [Self; 4] = [
        Self::System,
        Self::English,
        Self::ChineseSimplified,
        Self::Japanese,
    ];

    /// The saved name: `system`, `en`, `zh-CN` or `ja`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::English => "en",
            Self::ChineseSimplified => "zh-CN",
            Self::Japanese => "ja",
        }
    }

    /// A saved name, ignoring ASCII case. Anything else, including a missing
    /// value or a name from a later version, follows the system.
    #[must_use]
    pub fn parse(name: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|setting| setting.name().eq_ignore_ascii_case(name))
            .unwrap_or_default()
    }

    /// The language this choice shows, given the system's language tag.
    #[must_use]
    pub fn resolve(self, system_tag: &str) -> Language {
        match self {
            Self::System => system_language(system_tag),
            Self::English => Language::English,
            Self::ChineseSimplified => Language::ChineseSimplified,
            Self::Japanese => Language::Japanese,
        }
    }

    /// The choice's label in the settings panel when `language` is shown:
    /// System in that language, the others in their own.
    #[must_use]
    pub fn label_in(self, language: Language) -> String {
        match self {
            Self::System => strings_for(language).language_system.to_owned(),
            Self::English => Language::English.native_name().to_owned(),
            Self::ChineseSimplified => Language::ChineseSimplified.native_name().to_owned(),
            Self::Japanese => Language::Japanese.native_name().to_owned(),
        }
    }
}

/// The interface language for a system language tag such as `ja-JP`,
/// `zh-Hans-CN` or the POSIX `zh_CN.UTF-8`, by `LeanMark`'s rule: Japanese
/// for Japanese, Simplified Chinese for Simplified Chinese, and English for
/// everything else, Traditional Chinese (`zh-TW`, `zh-HK`, `zh-MO`,
/// `zh-Hant`) included.
#[must_use]
pub fn system_language(tag: &str) -> Language {
    let base = tag.split(['.', '@']).next().unwrap_or_default();
    let lower = base.replace('_', "-").to_ascii_lowercase();
    let is = |language: &str| {
        lower == language
            || lower
                .strip_prefix(language)
                .is_some_and(|rest| rest.starts_with('-'))
    };
    if is("ja") {
        return Language::Japanese;
    }
    if !is("zh") {
        return Language::English;
    }
    let rest = ["-", &lower[2..], "-"].concat();
    if rest.contains("-hant-") {
        Language::English
    } else if rest.contains("-hans-") {
        Language::ChineseSimplified
    } else if ["-tw-", "-hk-", "-mo-"]
        .iter()
        .any(|region| rest.contains(region))
    {
        Language::English
    } else {
        Language::ChineseSimplified
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// The language the window shows now.
#[must_use]
pub fn current() -> Language {
    Language::from_index(CURRENT.load(Ordering::Relaxed))
}

/// Shows `language` from now on. The window relabels its controls itself.
pub fn set_current(language: Language) {
    CURRENT.store(language.index(), Ordering::Relaxed);
}

/// The strings of the language shown now.
#[must_use]
pub fn strings() -> &'static Strings {
    strings_for(current())
}

/// The strings of `language`.
#[must_use]
pub const fn strings_for(language: Language) -> &'static Strings {
    match language {
        Language::English => &english::TABLE,
        Language::ChineseSimplified => &chinese::TABLE,
        Language::Japanese => &japanese::TABLE,
    }
}

/// The English strings, which the worker and the document engine write
/// their messages with.
#[must_use]
pub const fn english() -> &'static Strings {
    &english::TABLE
}

/// `template` with each `{name}` replaced by its value.
#[must_use]
pub fn fill(template: &str, values: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut text = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        text.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let value = after.find('}').and_then(|end| {
            let name = &after[..end];
            values
                .iter()
                .find(|(candidate, _)| *candidate == name)
                .map(|(_, value)| (end, value))
        });
        if let Some((end, value)) = value {
            use std::fmt::Write as _;
            let _ = write!(text, "{value}");
            rest = &after[end + 1..];
        } else {
            text.push('{');
            rest = after;
        }
    }
    text.push_str(rest);
    text
}

/// The text before and after the one `{name}` in `template`, so a string
/// built many times, such as a row's screen reader name, needs no search.
#[must_use]
pub fn around<'a>(template: &'a str, name: &str) -> (&'a str, &'a str) {
    let marker = ["{", name, "}"].concat();
    template.find(&marker).map_or((template, ""), |start| {
        (&template[..start], &template[start + marker.len()..])
    })
}

/// The keys of the messages that reach the window in English: from the
/// worker, the document engine, the clipboard and drop helpers, and the
/// reasons the shell gives inside a "... failed: {error}" status.
pub const MESSAGE_KEYS: &[&str] = &[
    "search_searching",
    "search_could_not_start",
    "search_navigation_failed",
    "search_first_match",
    "search_no_previous",
    "search_next",
    "search_source_changed",
    "search_result_unreadable",
    "search_failed",
    "search_match",
    "search_no_matches",
    "search_no_more",
    "search_quota",
    "sort_too_large",
    "sort_space_busy",
    "sort_disk_full",
    "sort_cancelled_reason",
    "error_path_required",
    "error_path_too_long",
    "error_remote_path",
    "error_device_path",
    "error_volume",
    "error_volume_check",
    "error_format",
    "error_not_file",
    "error_link",
    "error_changed",
    "error_untrusted",
    "error_cancelled",
    "error_find_xlsx",
    "error_sort_xlsx",
    "copy_too_many",
    "copy_range",
    "copy_not_cached",
    "copy_column_loading",
    "copy_too_large",
    "clipboard_busy",
    "drop_too_many",
    "find_empty",
    "find_no_document",
    "find_press_ctrl_f",
    "find_too_long",
    "status_tabs_limit",
    "status_row_not_indexed",
    "open_dialog_too_many",
    "open_dialog_failed",
    "column_only_data",
    "column_already_hidden",
    "column_last_visible",
    "column_open_first",
];

/// `message` in the language shown now.
#[must_use]
pub fn translate_message(message: &str) -> String {
    translate_message_for(current(), message)
}

/// `message`, written in English with one of the message templates, in
/// `language`. A message inside it as `{error}` is put into `language` too.
/// Anything else, such as text Windows wrote, comes back as it was.
#[must_use]
pub fn translate_message_for(language: Language, message: &str) -> String {
    translate_nested(language, message, 0)
}

fn translate_nested(language: Language, message: &str, depth: u8) -> String {
    if language == Language::English || depth > 3 {
        return message.to_owned();
    }
    for key in MESSAGE_KEYS {
        let (Some(template), Some(target)) = (english().get(key), strings_for(language).get(key))
        else {
            continue;
        };
        if let Some(values) = match_template(template, message) {
            let translated: Vec<(&str, String)> = values
                .into_iter()
                .map(|(name, value)| {
                    if name == "error" {
                        (name, translate_nested(language, value, depth + 1))
                    } else {
                        (name, value.to_owned())
                    }
                })
                .collect();
            let values: Vec<(&str, &dyn std::fmt::Display)> = translated
                .iter()
                .map(|(name, value)| (*name, value as &dyn std::fmt::Display))
                .collect();
            return fill(target, &values);
        }
    }
    message.to_owned()
}

/// The values of `template`'s placeholders if `message` is `template`
/// filled in.
fn match_template<'t, 'm>(template: &'t str, message: &'m str) -> Option<Vec<(&'t str, &'m str)>> {
    let mut literals = Vec::new();
    let mut names = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let end = rest[start..].find('}')? + start;
        literals.push(&rest[..start]);
        names.push(&rest[start + 1..end]);
        rest = &rest[end + 1..];
    }
    literals.push(rest);
    let mut remaining = message.strip_prefix(literals[0])?;
    let mut values = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        let literal = literals[index + 1];
        let last = index + 1 == names.len();
        let value_end = if last {
            if !remaining.ends_with(literal) {
                return None;
            }
            remaining.len() - literal.len()
        } else if literal.is_empty() {
            return None;
        } else {
            remaining.find(literal)?
        };
        values.push((*name, &remaining[..value_end]));
        remaining = &remaining[value_end + literal.len()..];
    }
    if names.is_empty() && !remaining.is_empty() {
        return None;
    }
    Some(values)
}

#[cfg(test)]
mod tests {
    use super::{Language, around, fill, match_template, set_current, strings};

    #[test]
    fn fill_replaces_known_names_and_keeps_others() {
        assert_eq!(fill("Zoom {percent}%", &[("percent", &125)]), "Zoom 125%");
        assert_eq!(fill("{a} {b} {a}", &[("a", &1)]), "1 {b} 1");
        assert_eq!(fill("no braces", &[]), "no braces");
        assert_eq!(
            fill("{unterminated", &[("unterminated", &1)]),
            "{unterminated"
        );
    }

    #[test]
    fn templates_match_only_their_own_shape() {
        assert_eq!(
            match_template("Match {number} ({count} stored).", "Match 3 (4 stored)."),
            Some(vec![("number", "3"), ("count", "4")])
        );
        assert_eq!(
            match_template("Match {number} ({count} stored).", "Match 3."),
            None
        );
        assert_eq!(
            match_template("No matches found.", "No matches found."),
            Some(vec![])
        );
        assert_eq!(
            match_template("No matches found.", "No matches found. Ever."),
            None
        );
        // The text after the last value must end the message.
        assert_eq!(
            match_template(
                "Search failed: {error}. Reopen.",
                "Search failed: a. Reopen. Later"
            ),
            None
        );
        assert_eq!(
            match_template(
                "Search failed: {error}. Reopen.",
                "Search failed: a. b. Reopen."
            ),
            Some(vec![("error", "a. b")])
        );
    }

    #[test]
    fn around_splits_at_one_placeholder() {
        assert_eq!(around("Row {row}", "row"), ("Row ", ""));
        assert_eq!(around("; Column {number}: ", "number"), ("; Column ", ": "));
        assert_eq!(around("plain", "row"), ("plain", ""));
    }

    #[test]
    fn the_default_language_is_english() {
        set_current(Language::English);
        assert_eq!(strings().status_ready, "Ready");
    }
}
