//! Print live and stopped rows from the foundation register.
//!
//! The register file is authoritative.

use std::process::ExitCode;

use register_index::{find_register, invocation, render, rows};

fn main() -> ExitCode {
    match report() {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("register-index: {error}");
            ExitCode::from(1)
        }
    }
}

fn report() -> Result<String, String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let parsed = invocation(&args).map_err(|error| error.to_string())?;
    let selection = parsed.selection;
    let path = match parsed.register {
        Some(path) => path,
        None => {
            let start = std::env::current_dir().map_err(|error| error.to_string())?;
            find_register(&start).ok_or_else(|| {
                "no docs/016-pre-wizzy-foundation.md at this directory or a parent; pass --register"
                    .to_string()
            })?
        }
    };
    let document = std::fs::read_to_string(&path)
        .map_err(|error| format!("reading {}: {error}", path.display()))?;
    let parsed_rows = rows(&document);
    let duplicates = register_index::duplicate_ids(&parsed_rows);
    if !duplicates.is_empty() {
        return Err(format!("duplicate register ids: {}", duplicates.join(", ")));
    }
    if parsed_rows.is_empty() {
        return Err(format!("no register rows in {}", path.display()));
    }
    Ok(render(&parsed_rows, selection))
}
