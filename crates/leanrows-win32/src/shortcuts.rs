//! Every keyboard shortcut and pointer action the settings panel lists,
//! grouped as `LeanMark` groups its own.

/// One row of the shortcut list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Shortcut {
    pub(crate) group: &'static str,
    pub(crate) keys: &'static str,
    pub(crate) action: &'static str,
}

const fn row(group: &'static str, keys: &'static str, action: &'static str) -> Shortcut {
    Shortcut {
        group,
        keys,
        action,
    }
}

/// The list in the order the panel shows it. A test checks it against the
/// accelerator table in `leanrows.rc`.
pub(crate) const SHORTCUTS: [Shortcut; 39] = [
    row("File", "Ctrl+O", "Open files"),
    row("File", "F5", "Reload the file"),
    row("Rows", "Ctrl+C", "Copy the selected rows"),
    row("Rows", "Ctrl+G", "Go to a row"),
    row("Rows", "Arrow keys", "Move through the rows"),
    row("Rows", "Page Up", "Up one page"),
    row("Rows", "Page Down", "Down one page"),
    row("Rows", "Ctrl++", "Zoom in"),
    row("Rows", "Ctrl+-", "Zoom out"),
    row("Rows", "Ctrl+0", "Actual size"),
    row("Rows", "Ctrl+wheel", "Zoom in or out"),
    row("Find", "Ctrl+F", "Find in the file"),
    row("Find", "F3", "Next match"),
    row("Find", "Shift+F3", "Previous match"),
    row("Find box", "Enter", "Next match"),
    row("Find box", "Shift+Enter", "Previous match"),
    row("Find box", "Esc", "Back to the rows"),
    row(
        "Sorting",
        "Click a column header",
        "Sort by it, again to reverse",
    ),
    row(
        "Sorting",
        "Right-click a column header",
        "Sort as text or numbers",
    ),
    row("Sorting", "Click Row", "Restore source order"),
    row("Sorting", "More menu", "Sort, restore, or cancel a sort"),
    row("Tabs", "Ctrl+Tab", "Next tab"),
    row("Tabs", "Ctrl+Shift+Tab", "Previous tab"),
    row("Tabs", "Ctrl+Page Down", "Next tab"),
    row("Tabs", "Ctrl+Page Up", "Previous tab"),
    row("Tabs", "Ctrl+1 to Ctrl+8", "Go to a tab by position"),
    row("Tabs", "Ctrl+9", "Go to the last tab"),
    row("Tabs", "Ctrl+W", "Close the tab"),
    row("Tabs", "Ctrl+F4", "Close the tab"),
    row("Tabs", "Middle click", "Close a tab"),
    row(
        "Tabs",
        "Left or right arrow",
        "Move between tabs in the strip",
    ),
    row("Window", "Ctrl+Shift+T", "Change the theme"),
    row("Window", "Ctrl+/", "Settings and shortcuts"),
    row("Window", "F1", "Settings and shortcuts"),
    row("Window", "Esc", "Close the settings panel"),
    row("Window", "Tab", "Next control"),
    row("Window", "Shift+Tab", "Previous control"),
    row("Window", "Up or down arrow", "Next theme in the panel"),
    row("Window", "Drop files", "Open them in tabs"),
];

/// The groups in the order they first appear.
#[must_use]
pub(crate) fn groups() -> Vec<&'static str> {
    let mut groups: Vec<&'static str> = Vec::new();
    for entry in SHORTCUTS {
        if !groups.contains(&entry.group) {
            groups.push(entry.group);
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::{SHORTCUTS, groups};
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
        let listed: BTreeSet<&str> = SHORTCUTS.iter().map(|entry| entry.keys).collect();
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
                .any(|entry| entry.group == group && entry.keys == keys)
        };
        assert!(find("Sorting", "Click a column header"));
        assert!(find("Sorting", "Right-click a column header"));
        assert!(find("Sorting", "Click Row"));
        assert!(find("Sorting", "More menu"));
        assert!(find("Find box", "Enter"));
        assert!(find("Find box", "Shift+Enter"));
        assert!(find("Find box", "Esc"));
        assert!(find("Window", "Esc"));
        assert!(find("Rows", "Ctrl+wheel"));
    }

    #[test]
    fn each_group_is_listed_once_and_in_order() {
        assert_eq!(
            groups(),
            [
                "File", "Rows", "Find", "Find box", "Sorting", "Tabs", "Window"
            ]
        );
        for entry in SHORTCUTS {
            assert!(!entry.keys.is_empty() && !entry.action.is_empty());
            assert!(!entry.keys.contains('&') && !entry.action.contains('&'));
            assert!(
                entry.keys.len() <= 32 && entry.action.len() <= 48,
                "{}",
                entry.keys
            );
        }
    }
}
