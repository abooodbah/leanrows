//! Checks the interface translations without a window, as `LeanMark`'s
//! `tests/Test-Localization.mjs` does: every language has every key and no
//! empty string, each translation keeps its placeholders and access keys,
//! nothing is left in English apart from product, format and key names, the
//! language follows the system unless the reader picked one, messages that
//! arrive in English are shown in the chosen language, and the native code
//! takes its visible text from the tables.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use leanrows_win32::AccessibleName;
use leanrows_win32::localization::{
    Language, LanguageSetting, MESSAGE_KEYS, strings_for, system_language, translate_message_for,
};

/// Names that stay in Latin letters in every language: the product, the
/// formats and limits it names, and the names printed on keys.
const LATIN_NAMES: &[&str] = &[
    "LeanRows",
    "Windows",
    "UTF-8",
    "ASCII",
    "non-ASCII",
    "CSV",
    "TSV",
    "JSONL",
    "NDJSON",
    "LOG",
    "TXT",
    "XLSX",
    ".xlsx",
    "UNC",
    "MiB",
    "64-bit",
    "Ctrl",
    "Shift",
    "Esc",
    "Enter",
    "Tab",
    "Page Up",
    "Page Down",
    "F1",
    "F3",
    "F5",
];

fn english() -> Vec<(&'static str, &'static str)> {
    strings_for(Language::English).entries()
}

fn placeholders(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            break;
        };
        found.insert(after[..end].to_owned());
        rest = &after[end + 1..];
    }
    found
}

/// The ASCII words left once placeholders, access keys and the names above
/// are taken out.
fn leftover_english(text: &str) -> Vec<String> {
    let mut rest = text.replace('&', "");
    for name in placeholders(text) {
        rest = rest.replace(&format!("{{{name}}}"), " ");
    }
    let mut names: Vec<&str> = LATIN_NAMES.to_vec();
    names.sort_by_key(|name| std::cmp::Reverse(name.len()));
    for name in names {
        rest = rest.replace(name, " ");
    }
    // A bracketed access key such as (&O) or (O) is a key letter, not a word.
    rest.split(|c: char| !c.is_ascii_alphabetic())
        .filter(|word| word.len() >= 2)
        .map(str::to_owned)
        .collect()
}

fn has_han(text: &str) -> bool {
    text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

fn has_kana(text: &str) -> bool {
    text.chars().any(|c| ('\u{3040}'..='\u{30ff}').contains(&c))
}

#[test]
fn every_language_has_every_key_and_no_empty_string() {
    let keys: Vec<&str> = english().iter().map(|(key, _)| *key).collect();
    assert!(keys.len() > 200, "only {} English strings", keys.len());
    for language in Language::ALL {
        let entries = strings_for(language).entries();
        let names: Vec<&str> = entries.iter().map(|(key, _)| *key).collect();
        assert_eq!(names, keys, "{} keys differ from English", language.tag());
        let empty: Vec<&str> = entries
            .iter()
            .filter(|(_, text)| text.trim().is_empty())
            .map(|(key, _)| *key)
            .collect();
        assert!(
            empty.is_empty(),
            "{} has empty strings: {empty:?}",
            language.tag()
        );
    }
}

#[test]
fn translations_keep_placeholders_and_access_keys() {
    let english = english();
    for language in [Language::ChineseSimplified, Language::Japanese] {
        let table = strings_for(language).entries();
        for ((key, source), (_, text)) in english.iter().zip(&table) {
            assert_eq!(
                placeholders(source),
                placeholders(text),
                "{} {key}: placeholders differ in {text:?}",
                language.tag()
            );
            let keys_in = source.matches('&').count();
            assert_eq!(
                keys_in,
                text.matches('&').count(),
                "{} {key}: access key count differs in {text:?}",
                language.tag()
            );
            if keys_in == 1 {
                let after = text.split('&').nth(1).and_then(|rest| rest.chars().next());
                assert!(
                    after.is_some_and(|c| c.is_ascii_alphabetic()),
                    "{} {key}: the access key must be a Latin letter, as in 開く(&O): {text:?}",
                    language.tag()
                );
            }
            // Pieces added to the status strip keep their separators.
            for edge in [" | ", "; ", " ("] {
                if source.starts_with(edge) {
                    assert!(
                        text.starts_with(edge),
                        "{} {key}: must start with {edge:?}: {text:?}",
                        language.tag()
                    );
                }
            }
            if source.ends_with(" | ") {
                assert!(
                    text.ends_with(" | "),
                    "{} {key}: must end with \" | \": {text:?}",
                    language.tag()
                );
            }
        }
    }
}

#[test]
fn nothing_is_left_in_english() {
    let english = english();
    for language in [Language::ChineseSimplified, Language::Japanese] {
        let table = strings_for(language).entries();
        let left: Vec<String> = english
            .iter()
            .zip(&table)
            .filter(|((_, source), (_, text))| {
                !leftover_english(text).is_empty() || (text == source && !source.is_empty())
            })
            .map(|((key, _), (_, text))| format!("{key}: {text}"))
            .collect();
        assert!(
            left.is_empty(),
            "{} strings left in English: {left:?}",
            language.tag()
        );
    }
}

#[test]
fn chinese_is_chinese_and_japanese_is_japanese() {
    let chinese = strings_for(Language::ChineseSimplified).entries();
    let no_han: Vec<&str> = chinese
        .iter()
        .filter(|(_, text)| !has_han(text))
        .map(|(key, _)| *key)
        .collect();
    assert!(no_han.is_empty(), "Chinese strings without Han: {no_han:?}");
    let kana: Vec<&str> = chinese
        .iter()
        .filter(|(_, text)| has_kana(text))
        .map(|(key, _)| *key)
        .collect();
    assert!(kana.is_empty(), "Chinese strings with kana: {kana:?}");

    let japanese = strings_for(Language::Japanese).entries();
    let with_kana = japanese.iter().filter(|(_, text)| has_kana(text)).count();
    assert!(
        with_kana * 10 > japanese.len() * 6,
        "only {with_kana} of {} Japanese strings have kana, so the table may be Chinese",
        japanese.len()
    );
    let not_japanese: Vec<&str> = japanese
        .iter()
        .filter(|(_, text)| !has_kana(text) && !has_han(text))
        .map(|(key, _)| *key)
        .collect();
    assert!(
        not_japanese.is_empty(),
        "Japanese strings without kana or kanji: {not_japanese:?}"
    );
}

#[test]
fn the_system_language_picks_the_interface() {
    for tag in ["ja", "ja-JP", "JA-jp", "ja_JP.UTF-8", "ja_JP", "ja-Jpan-JP"] {
        assert_eq!(system_language(tag), Language::Japanese, "{tag}");
    }
    // The same rule and tags as LeanMark's tests/Test-Localization.mjs:
    // Simplified Chinese tags pick Simplified Chinese, and Traditional
    // Chinese keeps English. Windows gives tags such as zh-CN, zh-SG, zh-TW,
    // zh-HK and zh-Hant-TW.
    for tag in [
        "zh-CN",
        "zh-cn",
        "zh",
        "zh-SG",
        "zh-Hans",
        "zh-Hans-CN",
        "zh-Hans-HK",
        "zh_CN.UTF-8",
        "zh_CN",
    ] {
        assert_eq!(system_language(tag), Language::ChineseSimplified, "{tag}");
    }
    for tag in [
        "en-US",
        "en",
        "",
        "de-DE",
        "fr-FR",
        "jax",
        "jv-ID",
        "zh-TW",
        "zh-HK",
        "zh-MO",
        "zh-Hant",
        "zh-Hant-TW",
        "zh-Hant-HK",
        "zh_TW.UTF-8",
        "ZH-tw",
        "zhx",
        "z",
        "-",
    ] {
        assert_eq!(system_language(tag), Language::English, "{tag:?}");
    }
    assert_eq!(system_language("ja@euro"), Language::Japanese);
}

#[test]
fn a_saved_choice_overrides_the_system() {
    assert_eq!(LanguageSetting::System.resolve("ja-JP"), Language::Japanese);
    assert_eq!(LanguageSetting::System.resolve("fr-FR"), Language::English);
    assert_eq!(LanguageSetting::English.resolve("zh-CN"), Language::English);
    assert_eq!(LanguageSetting::English.resolve("ja-JP"), Language::English);
    assert_eq!(
        LanguageSetting::ChineseSimplified.resolve("en-US"),
        Language::ChineseSimplified
    );
    assert_eq!(
        LanguageSetting::Japanese.resolve("zh-CN"),
        Language::Japanese
    );
    assert_eq!(
        LanguageSetting::ALL,
        [
            LanguageSetting::System,
            LanguageSetting::English,
            LanguageSetting::ChineseSimplified,
            LanguageSetting::Japanese
        ]
    );
    for setting in LanguageSetting::ALL {
        assert_eq!(LanguageSetting::parse(setting.name()), setting);
        assert_eq!(
            LanguageSetting::parse(&setting.name().to_ascii_uppercase()),
            setting
        );
    }
    assert_eq!(
        [
            LanguageSetting::System.name(),
            LanguageSetting::English.name(),
            LanguageSetting::ChineseSimplified.name(),
            LanguageSetting::Japanese.name()
        ],
        ["system", "en", "zh-CN", "ja"]
    );
    // A missing value, a name from a later version, or a near miss follows
    // the system, as an unknown theme does.
    for name in ["", "fr", "zh", "zh-TW", "english", "ja ", " en", "jap"] {
        assert_eq!(
            LanguageSetting::parse(name),
            LanguageSetting::System,
            "{name:?}"
        );
    }
    assert_eq!(
        [
            Language::English.tag(),
            Language::ChineseSimplified.tag(),
            Language::Japanese.tag()
        ],
        ["en", "zh-CN", "ja"]
    );
}

#[test]
fn the_choices_name_each_language_in_itself() {
    for language in Language::ALL {
        let labels: Vec<String> = LanguageSetting::ALL
            .iter()
            .map(|setting| setting.label_in(language))
            .collect();
        assert_eq!(
            labels[1..],
            ["English", "简体中文", "日本語"],
            "{}",
            language.tag()
        );
        assert_eq!(labels[0], strings_for(language).language_system);
    }
}

fn sample(name: &str) -> &'static str {
    match name {
        "error" => "disk on fire",
        "count" | "number" | "rows" | "row" | "total" => "7",
        "bytes" | "scanned" | "needed" | "limit" | "free" => "12",
        _ => "x",
    }
}

fn fill(template: &str) -> String {
    let mut text = template.to_owned();
    for name in placeholders(template) {
        text = text.replace(&format!("{{{name}}}"), sample(&name));
    }
    text
}

#[test]
fn messages_that_arrive_in_english_are_shown_in_the_chosen_language() {
    let english: Vec<(&str, &str)> = english();
    let lookup = |language: Language, key: &str| {
        strings_for(language)
            .entries()
            .into_iter()
            .find(|(name, _)| *name == key)
            .map(|(_, text)| text)
            .unwrap_or_default()
    };
    assert!(
        MESSAGE_KEYS.len() >= 40,
        "{} message keys",
        MESSAGE_KEYS.len()
    );
    for key in MESSAGE_KEYS {
        assert!(
            english.iter().any(|(name, _)| name == key),
            "{key} is not a key"
        );
        let message = fill(lookup(Language::English, key));
        for language in Language::ALL {
            assert_eq!(
                translate_message_for(language, &message),
                fill(lookup(language, key)),
                "{} {key}",
                language.tag()
            );
        }
    }
    // A message inside another is translated too.
    let inner = lookup(Language::English, "error_untrusted");
    let outer = lookup(Language::English, "search_source_changed").replace("{error}", inner);
    let expected = lookup(Language::Japanese, "search_source_changed")
        .replace("{error}", lookup(Language::Japanese, "error_untrusted"));
    assert_eq!(translate_message_for(Language::Japanese, &outer), expected);
    // Anything else, such as text Windows wrote, is shown as it came.
    for message in ["Access is denied. (os error 5)", "", "Ready later"] {
        assert_eq!(translate_message_for(Language::Japanese, message), message);
    }
    assert_eq!(
        translate_message_for(
            Language::English,
            &fill(lookup(Language::English, "search_match"))
        ),
        "Match 7 (7 stored)."
    );
}

fn source_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// The shipped code of one file: comments and `#[cfg(test)]` modules out.
fn shipped_code(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut kept = String::new();
    let mut skip_depth: Option<i32> = None;
    let mut depth = 0_i32;
    let mut pending_test = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with("#[cfg(test)]") {
            pending_test = true;
            continue;
        }
        let opens = i32::try_from(line.matches('{').count()).unwrap_or_default();
        let closes = i32::try_from(line.matches('}').count()).unwrap_or_default();
        if pending_test && skip_depth.is_none() {
            pending_test = false;
            if trimmed.starts_with("mod ") || trimmed.starts_with("pub(crate) mod ") {
                if trimmed.ends_with(';') {
                    continue;
                }
                skip_depth = Some(depth);
            }
        }
        depth += opens - closes;
        if let Some(start) = skip_depth {
            if depth <= start {
                skip_depth = None;
            }
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    kept
}

fn native_sources() -> Vec<PathBuf> {
    let root = source_root();
    let mut files = vec![root.join("native.rs"), root.join("shortcuts.rs")];
    if let Ok(entries) = std::fs::read_dir(root.join("native")) {
        let mut more: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().is_some_and(|extension| extension == "rs")
                    && !path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.ends_with("smoke.rs"))
            })
            .collect();
        more.sort();
        files.extend(more);
    }
    files
}

/// Wide literals that are names, not words a reader sees.
const NATIVE_NAMES: &[&str] = &[
    "LeanRows",
    "LR",
    "STATIC",
    "BUTTON",
    "EDIT",
    "Explorer",
    "LeanRowsScrollBar",
    "Software\\LeanRows",
    "Theme",
    "ZoomPercent",
    "Language",
    "AppsUseLightTheme",
    "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
];

#[test]
fn native_controls_menus_and_dialogs_take_their_text_from_the_tables() {
    let mut bare = Vec::new();
    for path in native_sources() {
        let code = shipped_code(&path);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        for (index, line) in code.lines().enumerate() {
            let mut rest = line;
            while let Some(start) = rest.find("w!(\"") {
                let after = &rest[start + 4..];
                let end = after.find('"').unwrap_or(after.len());
                let literal = &after[..end];
                // An escape such as \u{00d7} is a symbol, not a word.
                let words = !literal.starts_with('\\')
                    && literal.chars().filter(char::is_ascii_alphabetic).count() >= 2;
                let unescaped = literal.replace("\\\\", "\\");
                if words
                    && !NATIVE_NAMES.contains(&unescaped.as_str())
                    && !literal.starts_with("LeanRows.")
                    && !literal.starts_with("Local\\")
                {
                    bare.push(format!("{name}:{}: w!(\"{literal}\")", index + 1));
                }
                rest = &after[end.min(after.len())..];
            }
            // A status call whose text argument is an English literal or a
            // format string.
            for call in [
                "set_status(",
                "set_status_handle(",
                "set_control_text(",
                "set_tool_text(",
                "add_tool(",
                "name_control(",
            ] {
                if let Some(start) = line.find(call) {
                    let args = &line[start + call.len()..];
                    // The text is the last argument.
                    let text = args
                        .rsplit_once(',')
                        .map_or("", |(_, text)| text.trim_start());
                    // A format string counts when it has words outside its
                    // placeholders, so "{zoom}%" is not English.
                    let literal = text
                        .strip_prefix("&format!(\"")
                        .or_else(|| text.strip_prefix('"'))
                        .map(|rest| rest.split('"').next().unwrap_or_default());
                    let words = literal.is_some_and(|literal| {
                        let mut outside = String::new();
                        let mut depth = 0;
                        for c in literal.chars() {
                            match c {
                                '{' => depth += 1,
                                '}' => depth -= 1,
                                c if depth == 0 => outside.push(c),
                                _ => {}
                            }
                        }
                        outside.chars().filter(char::is_ascii_alphabetic).count() >= 2
                    });
                    if words {
                        bare.push(format!("{name}:{}: {}", index + 1, line.trim()));
                    }
                }
            }
        }
        // Calls split over lines: any argument line up to the closing
        // parenthesis that is a bare English literal.
        let lines: Vec<&str> = code.lines().collect();
        for pair in lines.windows(8) {
            let call = pair[0].trim_end();
            if [
                "set_status(",
                "set_status_handle(",
                "set_control_text(",
                "set_tool_text(",
                "add_tool(",
                "name_control(",
            ]
            .iter()
            .any(|c| call.ends_with(c))
            {
                for next in &pair[1..] {
                    let next = next.trim();
                    if next.starts_with(')') {
                        break;
                    }
                    if next.starts_with('"') || next.starts_with("&format!(\"") {
                        bare.push(format!("{name}: {call} {next}"));
                    }
                }
            }
        }
    }
    let shortcuts = shipped_code(&source_root().join("shortcuts.rs"));
    if shortcuts.contains("row(\"") {
        bare.push("shortcuts.rs: a shortcut row names its group or action in English".to_owned());
    }
    let theme = shipped_code(&source_root().join("native").join("theme.rs"));
    for label in ["=> \"System\"", "=> \"Light\"", "=> \"Dark\"", "\"Theme: {"] {
        if theme.contains(label) {
            bare.push(format!("theme.rs: {label}"));
        }
    }
    assert!(
        bare.is_empty(),
        "visible English outside the tables:\n{}",
        bare.join("\n")
    );
}

/// Every string the worker, the engine or the shell writes from the English
/// table, to be translated where it shows, is a message template.
#[test]
fn english_messages_are_message_templates() {
    let mut code = String::new();
    let root = source_root();
    let mut stack = vec![root.clone()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                code.push_str(&shipped_code(&path));
            }
        }
    }
    let joined: String = code.split_whitespace().collect();
    let mut missing = Vec::new();
    for (index, _) in joined.match_indices("english().") {
        let key: String = joined[index + "english().".len()..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let method = ["get", "entries"].contains(&key.as_str());
        if !method && !MESSAGE_KEYS.contains(&key.as_str()) && !missing.contains(&key) {
            missing.push(key);
        }
    }
    assert!(
        missing.is_empty(),
        "written in English but never translated: {missing:?}"
    );
}

/// A row's screen reader name is in the language shown. No other test here
/// reads the language shown, so changing it does not race them.
#[test]
fn row_names_follow_the_language_shown() {
    let name = |language: Language| {
        leanrows_win32::localization::set_current(language);
        let units = AccessibleName::compose(6, &[]).as_utf16().to_vec();
        String::from_utf16_lossy(&units[..units.len().saturating_sub(1)])
    };
    let japanese = name(Language::Japanese);
    let english = name(Language::English);
    assert_eq!(english, "Row 7");
    assert_eq!(
        japanese,
        strings_for(Language::Japanese)
            .accessible_row
            .replace("{row}", "7")
    );
}

#[test]
fn every_key_is_used_by_the_interface() {
    let mut code = String::new();
    let root = source_root();
    let mut stack = vec![root.clone()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name != "localization") {
                    stack.push(path);
                }
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                code.push_str(&shipped_code(&path));
            }
        }
    }
    code.push_str(&shipped_code(&root.join("localization.rs")));
    let app = Path::new(env!("CARGO_MANIFEST_DIR")).join("../leanrows-app/src/main.rs");
    code.push_str(&shipped_code(&app));
    let unused: Vec<&str> = english()
        .iter()
        .map(|(key, _)| *key)
        .filter(|key| !code.contains(&format!(".{key}")) && !code.contains(&format!("\"{key}\"")))
        .collect();
    assert!(unused.is_empty(), "keys no code uses: {unused:?}");
}
