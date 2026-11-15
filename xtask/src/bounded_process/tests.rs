#![cfg(windows)]

use super::*;

const PARKED_LOCK: &str = "WIZARD_PARKED_GRANDCHILD_LOCK";

const PARENT_LINGERS: &str = "WIZARD_PARKED_PARENT_LINGERS";

/// Holds its lock file exclusively for a minute unless its job ends it.
#[test]
#[ignore]
fn parked_grandchild_fixture() {
    let Some(path) = std::env::var_os(PARKED_LOCK) else {
        return;
    };
    use std::os::windows::fs::OpenOptionsExt;
    let _held = OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(path)
        .unwrap();
    std::thread::sleep(Duration::from_secs(60));
}

/// Starts the parked grandchild unawaited, sees its lock taken, then exits or lingers as asked.
#[test]
#[ignore]
fn grandparent_fixture_child() {
    let Some(path) = std::env::var_os(PARKED_LOCK).map(PathBuf::from) else {
        return;
    };
    let mut grandchild = Command::new(std::env::current_exe().unwrap());
    grandchild
        .args([
            "--exact",
            "bounded_process::tests::parked_grandchild_fixture",
            "--ignored",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Deliberately never awaited: the grandchild must outlive this process.
    drop(grandchild.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(20);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "the grandchild never took its lock"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    if std::env::var_os(PARENT_LINGERS).is_some() {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn parked_family(label: &str, lingers: bool) -> (PathBuf, PathBuf, Command) {
    let root = crate::test_support::reserve_temp_directory(&format!("wizard-parked-{label}"));
    let lock = root.join("parked.lock");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "bounded_process::tests::grandparent_fixture_child",
            "--ignored",
        ])
        .env(PARKED_LOCK, &lock);
    if lingers {
        command.env(PARENT_LINGERS, "1");
    }
    (root, lock, command)
}

/// The lock goes away only once no process holds it open.
fn released(lock: &std::path::Path) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if std::fs::remove_file(lock).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn a_completed_command_leaves_no_parked_descendant() {
    let (root, lock, command) = parked_family("completed", false);
    let result = output(command, Duration::from_secs(60), "grandparent fixture").unwrap();
    assert!(result.status.success(), "{result:?}");
    assert!(
        released(&lock),
        "a parked grandchild outlived its completed bounded command"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_timed_out_command_takes_its_parked_descendant_with_it() {
    let (root, lock, command) = parked_family("timed-out", true);
    let error = output(
        command,
        Duration::from_secs(8),
        "lingering grandparent fixture",
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("timed out"), "{error:#}");
    assert!(
        released(&lock),
        "a parked grandchild outlived its timed-out bounded command"
    );
    std::fs::remove_dir_all(root).unwrap();
}
