use std::process::Command;

#[test]
fn parse_refusals_fail_at_the_process_boundary() {
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .output()
        .expect("run xtask binary");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
}
