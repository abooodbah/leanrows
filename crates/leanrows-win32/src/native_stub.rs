use crate::{ShellError, ShellOptions, ShellOutcome};

pub(crate) fn run(_options: ShellOptions) -> Result<ShellOutcome, ShellError> {
    Err(ShellError::new(
        "the leanrows-win32 shell is available only on Windows",
    ))
}

pub(crate) fn show_startup_error(message: &str) {
    eprintln!("{message}");
}
