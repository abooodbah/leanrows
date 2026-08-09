#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    match leanrows_spike::run(std::env::args_os().skip(1)) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(error.exit_code())
        }
    }
}
