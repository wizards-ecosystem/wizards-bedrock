use super::*;

#[test]
fn only_the_reviewed_native_gdb_identity_is_accepted() {
    let version = format!("{GDB_IDENTITY}\nCopyright (C) 2025 Free Software Foundation\n");
    assert_eq!(require_identity(&version).unwrap(), GDB_IDENTITY);
    assert!(require_identity("").is_err());
    assert!(require_identity("GNU gdb (GDB) 17.2\n").is_err());
    assert!(require_identity(&version.replace("17.2", "17.3")).is_err());
}

#[test]
fn the_debugger_commands_are_entry_first_and_the_proof_is_bounded() {
    let elf = Path::new("kernel symbols.elf");
    let source = Path::new(r"C:\source checkout");
    let interactive = gdb_arguments(elf, "127.0.0.1:1234", source, DebugMode::Interactive).unwrap();
    let check = gdb_arguments(elf, "127.0.0.1:1234", source, DebugMode::Check).unwrap();
    let render = |arguments: &[OsString]| {
        arguments
            .iter()
            .map(|argument| argument.to_string_lossy())
            .collect::<Vec<_>>()
            .join("|")
    };
    let interactive = render(&interactive);
    let check = render(&check);

    for commands in [&interactive, &check] {
        assert!(commands.contains("--se|kernel symbols.elf"));
        assert!(commands.contains("set substitute-path /wizard/source \"C:/source checkout\""));
        assert!(commands.contains("target remote 127.0.0.1:1234"));
        assert!(commands.contains("hbreak kmain|--ex|continue"));
    }
    assert!(!interactive.contains("--batch"));
    assert!(!interactive.contains(PROOF_MARKER));
    assert!(check.contains("--batch"));
    assert!(check.contains(PROOF_MARKER));
    assert!(check.contains("backtrace|--ex|detach|--ex|quit"));
    assert!(IDENTITY_TIMEOUT <= Duration::from_secs(10));
    assert!(PROOF_TIMEOUT >= Duration::from_secs(10));
    assert!(PROOF_TIMEOUT <= Duration::from_secs(60));
}

#[test]
#[ignore]
fn slow_identity_child() {
    std::thread::sleep(Duration::from_secs(2));
}

#[test]
fn the_gdb_identity_probe_has_a_direct_child_return_deadline() {
    let mut command = Command::new(std::env::current_exe().expect("current test executable"));
    command.args([
        "--exact",
        "debugger::tests::slow_identity_child",
        "--ignored",
    ]);
    let started = Instant::now();
    let error = bounded_identity_output(command, Duration::from_millis(100)).unwrap_err();

    assert!(format!("{error:#}").contains("GDB identity probe timed out after 100 ms"));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn the_debugger_proof_requires_the_entry_marker_symbol_and_source() {
    let proof = "Breakpoint 1, wizard_kernel::kmain () at wost\\src\\boot/mod.rs:526\n\
                 wizard-debug: kernel-entry-hit\n\
                 #0  wizard_kernel::kmain () at wost\\src\\boot/mod.rs:526\n";
    require_proof(proof).unwrap();

    assert!(require_proof(&proof.replace(PROOF_MARKER, "wrong marker")).is_err());
    assert!(require_proof(&proof.replace("#0", "#1")).is_err());
    assert!(require_proof(&proof.replace("wizard_kernel::kmain", "kmain")).is_err());
    assert!(require_proof(&proof.replace("wost\\src\\boot/mod.rs", "unknown.rs")).is_err());
    assert!(require_proof(&format!("{proof}{PROOF_MARKER}\n")).is_err());
}
