//! Every keyboard shortcut and pointer action the settings panel lists,
//! grouped as `LeanMark` groups its own. The words come from the interface
//! strings; key names such as Ctrl+O are the same in every language.

use crate::localization::Strings;

/// One string of a language's table.
pub(crate) type Text = fn(&Strings) -> &'static str;

/// What a row shows in its key box: key names, or words for a gesture.
#[derive(Clone, Copy)]
pub(crate) enum Keys {
    Name(&'static str),
    Text(Text),
}

/// One row of the shortcut list.
#[derive(Clone, Copy)]
pub(crate) struct Shortcut {
    group: Text,
    keys: Keys,
    action: Text,
}

impl Shortcut {
    pub(crate) fn group(&self, strings: &Strings) -> &'static str {
        (self.group)(strings)
    }

    pub(crate) fn keys(&self, strings: &Strings) -> &'static str {
        match self.keys {
            Keys::Name(name) => name,
            Keys::Text(text) => text(strings),
        }
    }

    pub(crate) fn action(&self, strings: &Strings) -> &'static str {
        (self.action)(strings)
    }
}

const fn row(group: Text, keys: Keys, action: Text) -> Shortcut {
    Shortcut {
        group,
        keys,
        action,
    }
}

/// The list in the order the panel shows it. A test checks it against the
/// accelerator table in `leanrows.rc`.
pub(crate) const SHORTCUTS: [Shortcut; 41] = [
    row(
        |s| s.group_file,
        Keys::Name("Ctrl+O"),
        |s| s.action_open_files,
    ),
    row(|s| s.group_file, Keys::Name("F5"), |s| s.action_reload),
    row(
        |s| s.group_rows,
        Keys::Name("Ctrl+C"),
        |s| s.action_copy_rows,
    ),
    row(|s| s.group_rows, Keys::Name("Ctrl+G"), |s| s.action_goto),
    row(
        |s| s.group_rows,
        Keys::Text(|s| s.gesture_arrows),
        |s| s.action_move_rows,
    ),
    row(
        |s| s.group_rows,
        Keys::Name("Page Up"),
        |s| s.action_page_up,
    ),
    row(
        |s| s.group_rows,
        Keys::Name("Page Down"),
        |s| s.action_page_down,
    ),
    row(|s| s.group_rows, Keys::Name("Ctrl++"), |s| s.action_zoom_in),
    row(
        |s| s.group_rows,
        Keys::Name("Ctrl+-"),
        |s| s.action_zoom_out,
    ),
    row(
        |s| s.group_rows,
        Keys::Name("Ctrl+0"),
        |s| s.action_actual_size,
    ),
    row(
        |s| s.group_rows,
        Keys::Text(|s| s.gesture_ctrl_wheel),
        |s| s.action_zoom_wheel,
    ),
    row(|s| s.group_find, Keys::Name("Ctrl+F"), |s| s.action_find),
    row(|s| s.group_find, Keys::Name("F3"), |s| s.action_next_match),
    row(
        |s| s.group_find,
        Keys::Name("Shift+F3"),
        |s| s.action_previous_match,
    ),
    row(
        |s| s.group_find_box,
        Keys::Name("Enter"),
        |s| s.action_next_match,
    ),
    row(
        |s| s.group_find_box,
        Keys::Name("Shift+Enter"),
        |s| s.action_previous_match,
    ),
    row(
        |s| s.group_find_box,
        Keys::Name("Esc"),
        |s| s.action_back_to_rows,
    ),
    row(
        |s| s.group_sorting,
        Keys::Text(|s| s.gesture_click_header),
        |s| s.action_sort_header,
    ),
    row(
        |s| s.group_sorting,
        Keys::Text(|s| s.gesture_right_click_header),
        |s| s.action_sort_text_numbers,
    ),
    row(
        |s| s.group_sorting,
        Keys::Text(|s| s.gesture_click_row),
        |s| s.action_restore_order,
    ),
    row(
        |s| s.group_sorting,
        Keys::Text(|s| s.gesture_more_menu),
        |s| s.action_sort_menu,
    ),
    row(
        |s| s.group_columns,
        Keys::Text(|s| s.gesture_right_click_header),
        |s| s.action_hide_header,
    ),
    row(
        |s| s.group_columns,
        Keys::Text(|s| s.gesture_more_menu),
        |s| s.action_hide_menu,
    ),
    row(
        |s| s.group_tabs,
        Keys::Name("Ctrl+Tab"),
        |s| s.action_next_tab,
    ),
    row(
        |s| s.group_tabs,
        Keys::Name("Ctrl+Shift+Tab"),
        |s| s.action_previous_tab,
    ),
    row(
        |s| s.group_tabs,
        Keys::Name("Ctrl+Page Down"),
        |s| s.action_next_tab,
    ),
    row(
        |s| s.group_tabs,
        Keys::Name("Ctrl+Page Up"),
        |s| s.action_previous_tab,
    ),
    row(
        |s| s.group_tabs,
        Keys::Text(|s| s.gesture_tab_numbers),
        |s| s.action_tab_by_position,
    ),
    row(
        |s| s.group_tabs,
        Keys::Name("Ctrl+9"),
        |s| s.action_last_tab,
    ),
    row(
        |s| s.group_tabs,
        Keys::Name("Ctrl+W"),
        |s| s.action_close_tab,
    ),
    row(
        |s| s.group_tabs,
        Keys::Name("Ctrl+F4"),
        |s| s.action_close_tab,
    ),
    row(
        |s| s.group_tabs,
        Keys::Text(|s| s.gesture_middle_click),
        |s| s.action_close_a_tab,
    ),
    row(
        |s| s.group_tabs,
        Keys::Text(|s| s.gesture_left_right),
        |s| s.action_move_tabs,
    ),
    row(
        |s| s.group_window,
        Keys::Name("Ctrl+Shift+T"),
        |s| s.action_change_theme,
    ),
    row(
        |s| s.group_window,
        Keys::Name("Ctrl+/"),
        |s| s.action_settings,
    ),
    row(|s| s.group_window, Keys::Name("F1"), |s| s.action_settings),
    row(
        |s| s.group_window,
        Keys::Name("Esc"),
        |s| s.action_close_settings,
    ),
    row(
        |s| s.group_window,
        Keys::Name("Tab"),
        |s| s.action_next_control,
    ),
    row(
        |s| s.group_window,
        Keys::Name("Shift+Tab"),
        |s| s.action_previous_control,
    ),
    row(
        |s| s.group_window,
        Keys::Text(|s| s.gesture_up_down),
        |s| s.action_next_theme,
    ),
    row(
        |s| s.group_window,
        Keys::Text(|s| s.gesture_drop_files),
        |s| s.action_open_dropped,
    ),
];

/// The groups in the order they first appear, in `strings`' language.
#[must_use]
pub(crate) fn groups(strings: &Strings) -> Vec<&'static str> {
    let mut groups: Vec<&'static str> = Vec::new();
    for entry in SHORTCUTS {
        let group = entry.group(strings);
        if !groups.contains(&group) {
            groups.push(group);
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::{SHORTCUTS, groups};
    use crate::localization::english;
    use std::collections::BTreeSet;

    /// Reads the accelerator table the executable embeds and names each key
    /// the way the shortcut list writes it.
    fn embedded_accelerators() -> Vec<String> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../leanrows-app/resources/leanrows.rc"
        );
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let start = text.find("ACCELERATORS").unwrap_or(0);
        let block = &text[start..];
        let block = &block[block.find("BEGIN").unwrap_or(0) + 5..block.find("END").unwrap_or(0)];
        let mut names = Vec::new();
        for line in block.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let parts: Vec<&str> = line.split(',').map(str::trim).collect();
            let control = parts.contains(&"CONTROL");
            let shift = parts.contains(&"SHIFT");
            let key = match parts[0] {
                "0x09" => "Tab",
                "0x21" => "Page Up",
                "0x22" => "Page Down",
                "0x60" | "\"0\"" => "0",
                "0x6B" | "0xBB" => "+",
                "0x6D" | "0xBD" => "-",
                "0x70" => "F1",
                "0x72" => "F3",
                "0x73" => "F4",
                "0x74" => "F5",
                "0xBF" => "/",
                quoted => quoted.trim_matches('"'),
            };
            let name = if control && ["1", "2", "3", "4", "5", "6", "7", "8"].contains(&key) {
                "Ctrl+1 to Ctrl+8".to_owned()
            } else {
                // Shift is how "+" is typed on many keyboards; it is still Ctrl++.
                let shift = shift && key != "+";
                format!(
                    "{}{}{key}",
                    if control { "Ctrl+" } else { "" },
                    if shift { "Shift+" } else { "" }
                )
            };
            names.push(name);
        }
        names
    }

    #[test]
    fn every_embedded_shortcut_is_listed() {
        let accelerators = embedded_accelerators();
        assert!(accelerators.len() >= 30, "{accelerators:?}");
        let listed: BTreeSet<&str> = SHORTCUTS
            .iter()
            .map(|entry| entry.keys(english()))
            .collect();
        for name in &accelerators {
            assert!(listed.contains(name.as_str()), "{name} is not listed");
        }
        for name in ["Ctrl+/", "F1", "Ctrl+Shift+T", "Ctrl++", "Ctrl+-", "Ctrl+0"] {
            assert!(
                accelerators.iter().any(|a| a == name),
                "{name} is not embedded"
            );
        }
    }

    #[test]
    fn sorting_and_the_keys_the_window_handles_itself_are_listed() {
        let find = |group: &str, keys: &str| {
            SHORTCUTS
                .iter()
                .any(|entry| entry.group(english()) == group && entry.keys(english()) == keys)
        };
        assert!(find("Sorting", "Click a column header"));
        assert!(find("Sorting", "Right-click a column header"));
        assert!(find("Sorting", "Click Row"));
        assert!(find("Sorting", "More menu"));
        assert!(find("Columns", "Right-click a column header"));
        assert!(find("Columns", "More menu"));
        assert!(find("Find box", "Enter"));
        assert!(find("Find box", "Shift+Enter"));
        assert!(find("Find box", "Esc"));
        assert!(find("Window", "Esc"));
        assert!(find("Rows", "Ctrl+wheel"));
    }

    #[test]
    fn each_group_is_listed_once_and_in_order() {
        assert_eq!(
            groups(english()),
            [
                "File", "Rows", "Find", "Find box", "Sorting", "Columns", "Tabs", "Window"
            ]
        );
        for entry in SHORTCUTS {
            let (keys, action) = (entry.keys(english()), entry.action(english()));
            assert!(!keys.is_empty() && !action.is_empty());
            assert!(!keys.contains('&') && !action.contains('&'));
            assert!(keys.len() <= 32 && action.len() <= 48, "{keys}");
        }
    }
}
