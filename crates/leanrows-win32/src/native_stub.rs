use crate::{ShellError, ShellOptions, ShellOutcome};

pub(crate) fn run(_options: ShellOptions) -> Result<ShellOutcome, ShellError> {
    Err(ShellError::new(
        "the leanrows-win32 shell is available only on Windows",
    ))
}

pub(crate) fn free_disk_bytes(_path: &std::path::Path) -> std::io::Result<u64> {
    Ok(u64::MAX)
}

pub(crate) fn show_startup_error(message: &str) {
    eprintln!("{message}");
}
