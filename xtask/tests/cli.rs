//! The executable boundary owns exit status and diagnostic routing.

use std::path::Path;
use std::process::{Command, Output};

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(args)
        .current_dir(root)
        .output()
        .expect("run xtask binary")
}

#[test]
fn parse_refusals_fail_at_the_process_boundary() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits inside the workspace");
    let output = run(root, &[]);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("diagnostic is UTF-8");
    assert!(stderr.starts_with("error: no task given\n\ncargo xtask:"));
}
