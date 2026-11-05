//! Every keyboard shortcut and pointer action the settings panel lists,
//! grouped as `LeanMark` groups its own.

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
