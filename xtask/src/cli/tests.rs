use super::*;

fn parse_args(args: &[&str]) -> Result<Task> {
    parse(args.iter().map(OsString::from))
}

#[test]
fn unknown_tasks_and_options_are_rejected() {
    assert!(parse_args(&["bulid"]).is_err());
    assert!(parse_args(&["build", "--fast"]).is_err());
}
