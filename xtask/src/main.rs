#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    if let Some(code) = xtask::dispatch_private_codex_wrapper() {
        std::process::exit(code);
    }
    let args = std::env::args_os().skip(1);
    match xtask::cli::parse(args) {
        Ok(task) => match xtask::run(task) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error:?}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("error: {error}\n\n{}", xtask::cli::help());
            ExitCode::from(2)
        }
    }
}
