use super::*;

#[test]
fn the_machine_type_is_a_versioned_q35_not_an_alias() {
    let Some(version) = MACHINE_TYPE.strip_prefix("pc-q35-") else {
        panic!("the QEMU machine is not a q35 model");
    };
    let Some((major, minor)) = version.split_once('.') else {
        panic!("the QEMU machine has no explicit version");
    };
    assert!(
        !major.is_empty()
            && !minor.is_empty()
            && major.bytes().all(|byte| byte.is_ascii_digit())
            && minor.bytes().all(|byte| byte.is_ascii_digit())
    );
}

#[test]
fn the_fixed_qemu_dma_coverage_names_the_real_unit() {
    let table = qemu_dma_coverage_table();
    assert_eq!(
        table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)),
        0,
        "the complete ACPI table must carry a valid checksum"
    );
    let coverage = acpi_tables::DmaCoverage::new(&table)
        .expect("the fixed QEMU coverage attestation must parse");
    assert_eq!(coverage.segment(), 0);
    assert_eq!(coverage.register_base(), 0xfed9_0000);

    assert_eq!(
        acpi_file_argument(Path::new("ignored-parent/private,coverage.bin")),
        OsString::from("file=private,,coverage.bin"),
        "QEMU must receive a cwd-relative file name, not a Windows drive path"
    );
}

#[test]
fn every_qemu_run_carries_the_exact_dma_handoff_authority() {
    let table = crate::platform_acpi::dma_handoff_table();
    let handoff =
        acpi_tables::DmaHandoff::new(&table).expect("the fixed QEMU handoff authority must parse");
    assert_eq!(handoff.segment(), 0);
    assert_eq!(
        table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)),
        0
    );
}

#[test]
fn every_qemu_runner_pins_the_exact_tcg_profile() {
    let machine = machine_args(Remapping::Present);
    assert_eq!(
        machine
            .iter()
            .filter(|argument| **argument == "-no-user-config")
            .count(),
        1,
        "ambient QEMU configuration can rewrite the fixed profile"
    );

    let accelerator: Vec<_> = machine
        .windows(2)
        .filter_map(|pair| (pair[0] == "-accel").then_some(pair[1]))
        .collect();
    assert_eq!(accelerator, ["tcg"]);

    let cpu: Vec<_> = machine
        .windows(2)
        .filter_map(|pair| (pair[0] == "-cpu").then_some(pair[1]))
        .collect();
    assert_eq!(cpu, ["max,hypervisor=on,tcg-cpuid=on"]);

    // The unit precedes every PCI device, and only the unisolated boot
    // leaves it out; nothing else about the two machines differs.
    let devices: Vec<_> = machine
        .windows(2)
        .filter_map(|pair| (pair[0] == "-device").then_some(pair[1]))
        .collect();
    assert_eq!(
        devices,
        [
            "intel-iommu,intremap=on,eim=on,caching-mode=on",
            "pci-bridge,id=behind,chassis_nr=1"
        ]
    );
    let unisolated = machine_args(Remapping::Absent);
    let unit = machine
        .iter()
        .position(|argument| *argument == IOMMU_DEVICE[1])
        .expect("the ordinary machine carries the unit");
    let mut without_unit = machine.clone();
    without_unit.drain(unit - 1..=unit);
    assert_eq!(unisolated, without_unit);
    assert_eq!(
        DEGRADED_CPU,
        [
            "-cpu",
            "qemu64,-x2apic,-smep,-smap,-umip,hypervisor=on,tcg-cpuid=on"
        ]
    );

    let source = concat!(
        include_str!("../debug.rs"),
        include_str!("../run_files.rs"),
        include_str!("../storage.rs"),
        include_str!("../serial.rs"),
        include_str!("../selftest.rs"),
        include_str!("../boot_check.rs"),
        include_str!("../../qemu.rs")
    );
    let production = source
        .split_once("#[cfg(test)]")
        .expect("QEMU test module")
        .0;
    let command = ["Command::new", "(&installation.emulator)"].concat();
    let profile = [".args", "(machine_args("].concat();
    let commands = production.matches(&command).count();
    assert_ne!(commands, 0, "no QEMU process construction was found");
    assert_eq!(
        production.matches(&profile).count(),
        commands,
        "a QEMU process bypasses the shared machine profile"
    );
    let working_directory = [".current_dir", "(vars.directory())"].concat();
    assert_eq!(
        production.matches(&working_directory).count(),
        commands,
        "a QEMU process can resolve the cwd-relative coverage table elsewhere"
    );
    let direct_coverage = [".args", "(coverage.arguments())"].concat();
    assert_eq!(
        production.matches(&direct_coverage).count(),
        commands,
        "a QEMU process bypasses the explicit platform ACPI authorities"
    );
    assert_eq!(
        production.matches("RunDmaCoverage::create").count(),
        commands,
        "a QEMU process has no per-run handoff-table owner"
    );
    let owner = production
        .split_once("impl RunDmaCoverage")
        .expect("platform ACPI owner")
        .1
        .split_once("fn write_run_acpi_table")
        .expect("platform ACPI owner end")
        .0;
    assert!(owner.contains("crate::platform_acpi::dma_handoff_table()"));
    assert!(owner.contains("remapping == Remapping::Present"));
}

#[test]
fn interactive_arguments_cannot_replace_the_tcg_profile() {
    for conflict in [
        vec!["-accel", "whpx"],
        vec!["-accel=whpx"],
        vec!["--accel", "whpx"],
        vec!["--accel=whpx"],
        vec!["-enable-kvm"],
        vec!["--enable-kvm=true"],
        vec!["-machine", "pc-q35-9.2,accel=whpx"],
        vec!["-machine", "accel=whpx"],
        vec!["-machine=pc-q35-9.2,accel=whpx"],
        vec!["--machine", "accel=whpx"],
        vec!["--machine=pc-q35-9.2,accel=whpx"],
        vec!["-M", "pc"],
        vec!["-M", "accel=whpx"],
        vec!["-M=pc"],
        vec!["-cpu", "host"],
        vec!["-cpu=host"],
        vec!["--cpu", "host"],
        vec!["--cpu=max,hv-vendor-id=TCGTCGTCGTCG"],
        vec!["-global", "max-x86_64-cpu.hypervisor=off"],
        vec!["-global=max-x86_64-cpu.tcg-cpuid=off"],
        vec![
            "--global",
            "driver=max-x86_64-cpu,property=hypervisor,value=off",
        ],
        vec!["-acpitable", "file=hostile.bin"],
        vec!["-acpitable=file=hostile.bin"],
        vec!["--acpitable", "file=hostile.bin"],
        vec!["--acpitable=file=hostile.bin"],
        vec!["-readconfig", "hostile.cfg"],
        vec!["-readconfig=hostile.cfg"],
        vec!["--readconfig", "hostile.cfg"],
        vec!["-device", "vfio-pci,host=01:00.0"],
        vec!["-device=vfio-pci,host=01:00.0"],
        vec!["-netdev", "user,id=changed"],
        vec!["-incoming", "defer"],
        vec!["-drive", "file=hostile.img"],
        vec!["-bios", "hostile.bin"],
        vec!["-set", "drive.disk.snapshot=off"],
        vec!["--set", "device.nvme0.drive=hostile"],
    ] {
        let conflict: Vec<_> = conflict.into_iter().map(OsString::from).collect();
        let error = validate_run_arguments(&conflict).unwrap_err();
        assert!(error.to_string().contains("fixed TCG machine profile"));
    }

    let diagnostic: Vec<_> = [
        "-d",
        "int",
        "-D=qemu-debug.log",
        "-display",
        "gtk",
        "--display=none",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    validate_run_arguments(&diagnostic).unwrap();

    let source = include_str!("../../qemu.rs");
    let start = source
        .find("pub fn run(paths: &Paths, image: &Path, extra: &[OsString])")
        .expect("interactive QEMU runner");
    let run = &source[start..];
    let validation = run
        .find(&["validate_run_arguments", "(extra)?;"].concat())
        .expect("interactive argument validation");
    let installation = run.find("let installation = installation()?;").unwrap();
    let forwarding = run.find(".args(extra)").unwrap();
    assert!(validation < installation && installation < forwarding);
}

#[test]
fn exe_name_carries_the_platform_suffix() {
    let name = exe_name("qemu-system-x86_64");
    assert!(name.ends_with(std::env::consts::EXE_SUFFIX));
    assert!(name.starts_with("qemu-system-x86_64"));
}

#[test]
fn which_in_finds_a_binary_that_is_there() {
    let this = std::env::current_exe().unwrap();
    let dir = this.parent().unwrap().to_path_buf();
    let name = this.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(which_in([dir], &name), Some(this));
}

#[test]
fn which_in_reports_a_binary_that_is_not() {
    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    assert_eq!(which_in([dir], "definitely-not-installed-xyz"), None);
}

#[test]
fn installation_preserves_the_explicit_binary_and_its_firmware() {
    let fixture = QemuFixture::create();
    let binary = fixture.root.join(exe_name("selected-qemu"));
    let code = fixture.root.join(CODE_FIRMWARE);
    let vars = fixture.root.join(VARS_FIRMWARE);
    std::fs::write(&binary, b"selected emulator").unwrap();
    std::fs::write(&code, b"code firmware").unwrap();
    std::fs::write(&vars, b"variable firmware").unwrap();
    let ignored = fixture.root.join("ignored installation");
    let selected =
        installation_with(Some(binary.clone()), Some(ignored.clone()), [ignored]).unwrap();
    assert_eq!(selected.emulator, binary);
    assert_eq!(selected.code_firmware, code);
    assert_eq!(selected.vars_firmware, vars);
}

#[test]
fn installation_finds_firmware_under_an_fhs_prefix() {
    let fixture = QemuFixture::create();
    let binary_dir = fixture.root.join("bin");
    let firmware_dir = fixture.root.join("share").join("qemu");
    std::fs::create_dir_all(&binary_dir).unwrap();
    std::fs::create_dir_all(&firmware_dir).unwrap();
    let binary = binary_dir.join(exe_name("qemu-system-x86_64"));
    let code = firmware_dir.join(CODE_FIRMWARE);
    let vars = firmware_dir.join(VARS_FIRMWARE);
    std::fs::write(&binary, b"fhs emulator").unwrap();
    std::fs::write(&code, b"code firmware").unwrap();
    std::fs::write(&vars, b"variable firmware").unwrap();
    let selected =
        installation_with(Some(binary.clone()), None, std::iter::empty::<PathBuf>()).unwrap();
    assert_eq!(selected.emulator, binary);
    assert_eq!(selected.code_firmware, code);
    assert_eq!(selected.vars_firmware, vars);
}

#[test]
fn a_missing_qemu_setting_is_not_blamed_on_firmware() {
    let fixture = QemuFixture::create();
    let missing = fixture.root.join(exe_name("qemu-system-x86_64"));
    let explicit = installation_with(Some(missing), None, std::iter::empty::<PathBuf>())
        .unwrap_err()
        .to_string();
    assert!(explicit.starts_with("QEMU names "), "{explicit}");
    let directory = installation_with(
        None,
        Some(fixture.root.clone()),
        std::iter::empty::<PathBuf>(),
    )
    .unwrap_err()
    .to_string();
    assert_eq!(
        directory,
        format!(
            "QEMU_DIR names {}, which holds no {}",
            fixture.root.display(),
            exe_name("qemu-system-x86_64")
        )
    );
    let advice = "set QEMU to the executable or QEMU_DIR to its installation";
    let undefaulted = locate(None, None, None, std::iter::empty::<PathBuf>())
        .unwrap_err()
        .to_string();
    assert_eq!(
        undefaulted,
        format!(
            "QEMU not found: looked for {} on PATH; {advice}",
            exe_name("qemu-system-x86_64")
        )
    );
    let defaulted = locate(
        None,
        None,
        Some(&fixture.root),
        std::iter::empty::<PathBuf>(),
    )
    .unwrap_err()
    .to_string();
    assert_eq!(
        defaulted,
        format!(
            "QEMU not found: looked for {} in {} and on PATH; {advice}",
            exe_name("qemu-system-x86_64"),
            fixture.root.display()
        )
    );
}

#[test]
fn drive_paths_escape_commas_without_quoting_spaces() {
    assert_eq!(
        drive_file("format=raw", Path::new("a space/disk,name.img")),
        [
            OsString::from("-drive"),
            OsString::from("format=raw,file=a space/disk,,name.img")
        ]
    );
}

#[test]
fn shared_media_writes_use_a_private_snapshot() {
    assert_eq!(
        snapshot_drive_file("format=raw", Path::new("disk.img")),
        [
            OsString::from("-drive"),
            OsString::from("format=raw,snapshot=on,file=disk.img")
        ]
    );
}

#[test]
fn qemu_control_stays_on_the_child_s_own_streams() {
    assert_eq!(QMP_ON_STDIO, ["-qmp", "stdio"]);
    let source = concat!(
        include_str!("../debug.rs"),
        include_str!("../run_files.rs"),
        include_str!("../storage.rs"),
        include_str!("../serial.rs"),
        include_str!("../selftest.rs"),
        include_str!("../boot_check.rs"),
        include_str!("../../qemu.rs")
    );
    let (production, _) = source
        .split_once("#[cfg(test)]\nmod tests")
        .expect("qemu.rs has its test module");
    for forbidden in ["TcpListener", "\"-monitor\"", "server=off"] {
        assert!(
            !production.contains(forbidden),
            "QEMU control regained `{forbidden}`"
        );
    }
}

#[test]
fn the_debug_endpoint_is_allocated_and_loopback_only() {
    let report =
        "#chr072: filename=gdb\ngdb: filename=disconnected:tcp:127.0.0.1:56552,server=on\n(qemu)";
    assert_eq!(parse_debug_endpoint(report).unwrap(), "127.0.0.1:56552");

    for refused in [
        "serial0: filename=null",
        "gdb: filename=disconnected:tcp:0.0.0.0:56552,server=on",
        "gdb: filename=disconnected:tcp:127.0.0.1:0,server=on",
        "gdb: filename=disconnected:tcp:127.0.0.1:not-a-port,server=on",
    ] {
        assert!(parse_debug_endpoint(refused).is_err(), "accepted {refused}");
    }
}

#[test]
fn simultaneous_runs_get_distinct_writable_firmware() {
    let fixture = QemuFixture::create();
    let target = Arc::new(fixture.root.join("target"));
    let template = Arc::new(fixture.template.clone());
    let barrier = Arc::new(std::sync::Barrier::new(3));

    let first = {
        let target = Arc::clone(&target);
        let template = Arc::clone(&template);
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            RunVars::create(&target, &template).expect("first private variable store")
        })
    };
    let second = {
        let target = Arc::clone(&target);
        let template = Arc::clone(&template);
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            RunVars::create(&target, &template).expect("second private variable store")
        })
    };
    barrier.wait();
    let first = first.join().expect("first firmware creator");
    let second = second.join().expect("second firmware creator");

    assert_ne!(first.path(), second.path());
    assert_eq!(
        std::fs::read(first.path()).unwrap(),
        b"pristine firmware variables"
    );
    assert_eq!(
        std::fs::read(second.path()).unwrap(),
        b"pristine firmware variables"
    );
    let first_path = first.path().to_path_buf();
    let second_path = second.path().to_path_buf();
    drop(first);
    drop(second);
    assert!(!first_path.exists());
    assert!(!second_path.exists());
}

#[test]
fn private_firmware_refuses_a_redirected_run_root_before_writing() {
    let fixture = QemuFixture::create();
    let target = fixture.root.join("target");
    let outside = fixture.root.join("outside");
    std::fs::create_dir(&target).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("sentinel"), b"outside authority").unwrap();
    let run_root = target.join(crate::paths::QEMU_RUN_DIRECTORY);
    create_directory_symlink(&outside, &run_root);
    let reserved_outside = std::cell::Cell::new(false);

    assert!(
        RunVars::create_with(&target, &fixture.template, |_| reserved_outside.set(true)).is_err()
    );
    assert!(
        !reserved_outside.get(),
        "private firmware reserved an external output before refusal"
    );
    assert_eq!(
        crate::paths::directory_entries(&outside).unwrap().len(),
        1,
        "private firmware creation escaped its configured target"
    );
    assert_eq!(
        std::fs::read(outside.join("sentinel")).unwrap(),
        b"outside authority"
    );
    crate::test_support::remove_directory_link(&run_root);
}

#[test]
fn storage_construction_is_serialized_across_runners() {
    let fixture = QemuFixture::create();
    let lock_path = fixture.root.join("nvme.lock");
    let canonical = crate::mutate::workspace::canonical_existing_real_directory(
        &fixture.root,
        "storage fixture",
    )
    .unwrap();
    let held =
        acquire_storage_lock(&fixture.root, &lock_path, &canonical).expect("first storage builder");
    let (contended_tx, contended_rx) = std::sync::mpsc::sync_channel(0);
    let (continue_tx, continue_rx) = std::sync::mpsc::sync_channel(0);
    let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
    let parent = fixture.root.clone();
    let canonical = canonical.clone();
    let contender = std::thread::spawn(move || {
        let _lock = acquire_storage_lock_with(&parent, &lock_path, &canonical, || {
            contended_tx.send(()).expect("announce contention");
            continue_rx.recv().expect("wait for release");
        })
        .expect("second storage builder");
        acquired_tx.send(()).expect("announce acquisition");
    });

    contended_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("contender observed the held lock");
    drop(held);
    continue_tx.send(()).expect("permit retry after release");
    acquired_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("contender acquired released lock");
    contender.join().expect("storage contender");
}

#[test]
fn an_open_storage_lock_must_remain_at_its_name() {
    let fixture = QemuFixture::create();
    let lock_path = fixture.root.join("nvme.lock");
    let canonical = crate::mutate::workspace::canonical_existing_real_directory(
        &fixture.root,
        "storage fixture",
    )
    .unwrap();

    let error = open_storage_lock_with(&fixture.root, &lock_path, &canonical, |opened| {
        std::fs::remove_file(opened).expect("unlink the opened lock")
    })
    .expect_err("an unlinked lock cannot serialize later builders");

    assert!(
        error
            .to_string()
            .contains("disappeared after it was opened")
    );
}

#[test]
fn storage_publication_never_writes_through_an_external_link() {
    let fixture = QemuFixture::create();
    let parent = fixture.root.join("storage");
    let outside = fixture.root.join("outside");
    std::fs::create_dir(&parent).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let sentinel = outside.join("sentinel");
    let backing = parent.join("nvme.img");
    let lock = parent.join("nvme.lock");
    std::fs::write(&sentinel, b"outside authority").unwrap();

    std::fs::hard_link(&sentinel, &backing).unwrap();
    assert_eq!(publish_fixture(&parent, b"new namespace").unwrap(), backing);
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
    assert_eq!(std::fs::read(&backing).unwrap(), b"new namespace");

    std::fs::remove_file(&backing).unwrap();
    std::fs::write(&sentinel, b"same namespace").unwrap();
    std::fs::hard_link(&sentinel, &backing).unwrap();
    publish_fixture(&parent, b"same namespace").unwrap();
    std::fs::write(&backing, b"private namespace").unwrap();
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"same namespace");

    std::fs::remove_file(&backing).unwrap();
    std::fs::write(&sentinel, b"outside authority").unwrap();
    create_file_symlink(&sentinel, &backing);
    assert!(publish_fixture(&parent, b"forged").is_err());
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
    std::fs::remove_file(&backing).unwrap();

    std::fs::remove_file(&lock).unwrap();
    std::fs::hard_link(&sentinel, &lock).unwrap();
    publish_fixture(&parent, b"lock hardlink").unwrap();
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
    std::fs::remove_file(&backing).unwrap();
    std::fs::remove_file(&lock).unwrap();

    create_file_symlink(&sentinel, &lock);
    assert!(publish_fixture(&parent, b"lock symlink").is_err());
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
    std::fs::remove_file(&lock).unwrap();
}

#[test]
fn storage_creation_refuses_a_redirected_parent_before_writing() {
    let fixture = QemuFixture::create();
    let target = fixture.root.join("target");
    let outside = fixture.root.join("outside-storage");
    std::fs::create_dir(&target).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("sentinel"), b"outside authority").unwrap();
    let storage = target.join("qemu storage,private");
    create_directory_symlink(&outside, &storage);

    assert!(storage_paths(&fixture.root).is_err());
    assert_eq!(crate::paths::directory_entries(&outside).unwrap().len(), 1);
    assert_eq!(
        std::fs::read(outside.join("sentinel")).unwrap(),
        b"outside authority"
    );
    crate::test_support::remove_directory_link(&storage);
}

#[test]
fn a_failed_storage_stage_preserves_the_complete_authoritative_image() {
    let fixture = QemuFixture::create();
    let parent = fixture.root.join("storage");
    std::fs::create_dir(&parent).unwrap();
    let backing = publish_fixture(&parent, b"complete old image").unwrap();

    let error = publish_namespace(&parent, &backing, |file| {
        file.write_all(b"partial")?;
        bail!("injected construction failure")
    })
    .unwrap_err();
    assert!(error.to_string().contains("injected construction failure"));
    assert_eq!(std::fs::read(&backing).unwrap(), b"complete old image");
    assert!(!parent.join(STORAGE_STAGING_NAME).exists());
}

#[test]
fn storage_staging_supports_read_modify_write_builders() {
    let fixture = QemuFixture::create();
    let parent = fixture.root.join("storage");
    std::fs::create_dir(&parent).unwrap();
    let backing = parent.join("nvme.img");

    publish_namespace(&parent, &backing, |file| {
        file.write_all(b"format input")?;
        Seek::seek(&mut *file, SeekFrom::Start(0))?;
        let mut observed = Vec::new();
        file.read_to_end(&mut observed)?;
        ensure!(observed == b"format input", "staging handle changed bytes");
        Ok(())
    })
    .unwrap();
    assert_eq!(std::fs::read(backing).unwrap(), b"format input");
}

#[test]
fn the_nvme_namespace_is_streamed_to_its_exact_extent() {
    let fixture = QemuFixture::create();
    let path = fixture.root.join("streamed-nvme.img");
    let mut options = OpenOptions::new();
    OpenOptions::create_new(&mut options, true);
    OpenOptions::read(&mut options, true);
    OpenOptions::write(&mut options, true);
    let mut file = OpenOptions::open(&options, &path).unwrap();
    write_nvme_namespace(&mut file).unwrap();
    assert_eq!(file.metadata().unwrap().len(), NVME_BYTES);
    for lba in 0..NVME_SIGNED_BLOCKS {
        let mut signature = [0u8; 8];
        Seek::seek(&mut file, SeekFrom::Start(lba * NVME_BLOCK_BYTES as u64)).unwrap();
        file.read_exact(&mut signature).unwrap();
        assert_eq!(u64::from_le_bytes(signature), nvme_signature(lba));
    }
    let mut sector = [0u8; fat32::SECTOR_BYTES];
    Seek::seek(
        &mut file,
        SeekFrom::Start(NVME_VOLUME_SECTOR * NVME_BLOCK_BYTES as u64),
    )
    .unwrap();
    file.read_exact(&mut sector).unwrap();
    fat32::Bpb::parse(&sector).expect("streamed FAT32 corpus");
}

#[test]
fn qemu_stop_polls_until_a_terminated_child_is_reaped() {
    let mut child = fake_qemu();
    child.exit_after_kill_polls = Some(3);
    stop_qemu_with(
        &mut child,
        FakeQemuChild::exited,
        FakeQemuChild::terminate,
        Duration::from_secs(1),
    )
    .unwrap();
    assert!(child.killed);
    assert_eq!(child.polls_after_kill, 3);

    let mut exited = fake_qemu();
    exited.exited = true;
    stop_qemu_with(
        &mut exited,
        FakeQemuChild::exited,
        FakeQemuChild::terminate,
        Duration::ZERO,
    )
    .unwrap();
    assert!(!exited.killed);
}

struct LifetimeProbe {
    cancelled: Option<Arc<AtomicBool>>,
    stops: usize,
    refuse: bool,
    observed_cancelled: bool,
}

fn stop_lifetime_probe(probe: &mut &mut LifetimeProbe) -> Result<()> {
    probe.observed_cancelled = probe
        .cancelled
        .as_ref()
        .is_some_and(|flag| flag.load(Ordering::Acquire));
    probe.stops += 1;
    ensure!(!probe.refuse, "fixture stop refusal");
    Ok(())
}

#[test]
fn selftest_lifetime_cancels_and_stops_on_return_error_and_unwind() {
    for early_error in [false, true] {
        let mut probe = LifetimeProbe {
            cancelled: None,
            stops: 0,
            refuse: false,
            observed_cancelled: false,
        };
        let outcome: Result<()> = (|| {
            let owner = QemuLifetime::new(&mut probe, stop_lifetime_probe);
            owner.child.cancelled = Some(Arc::clone(&owner.cancelled));
            if early_error {
                bail!("fixture poll or injector failure");
            }
            Ok(())
        })();
        assert_eq!(outcome.is_err(), early_error);
        assert_eq!(probe.stops, 1);
        assert!(probe.observed_cancelled);
    }
    let mut probe = LifetimeProbe {
        cancelled: None,
        stops: 0,
        refuse: false,
        observed_cancelled: false,
    };
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let owner = QemuLifetime::new(&mut probe, stop_lifetime_probe);
        owner.child.cancelled = Some(Arc::clone(&owner.cancelled));
        panic!("fixture unwind");
    }));
    assert!(panicked.is_err());
    assert_eq!(probe.stops, 1);
    assert!(probe.observed_cancelled);
}

#[test]
fn selftest_lifetime_reports_stop_refusal_and_preserves_both_errors() {
    let mut probe = LifetimeProbe {
        cancelled: None,
        stops: 0,
        refuse: true,
        observed_cancelled: false,
    };
    let cleanup = {
        let mut owner = QemuLifetime::new(&mut probe, stop_lifetime_probe);
        owner.child.cancelled = Some(Arc::clone(&owner.cancelled));
        owner.stop()
    };
    assert_eq!(
        probe.stops, 1,
        "a refused stop was retried without a new bound"
    );
    let error = combine_qemu_cleanup::<()>(Err(anyhow!("fixture observation failure")), cleanup)
        .unwrap_err()
        .to_string();
    assert!(error.contains("fixture observation failure"));
    assert!(error.contains("fixture stop refusal"));
    assert!(combine_qemu_cleanup(Ok(()), Err(anyhow!("cleanup"))).is_err());
    assert!(combine_qemu_cleanup::<()>(Err(anyhow!("observation")), Ok(())).is_err());
    assert_eq!(combine_qemu_cleanup(Ok(17), Ok(())).unwrap(), 17);
}

// The worker cannot self-release before the parent is ready to exercise the
// join. A separate watchdog bounds a broken join without the old 150 ms race.
fn with_blocked_worker<T: Send + 'static, R>(value: T, join: impl FnOnce(JoinHandle<T>) -> R) -> R {
    let (release, parked) = std::sync::mpsc::channel();
    let (entered, ready) = std::sync::mpsc::channel();
    let (acknowledge, done) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        entered.send(()).unwrap();
        parked.recv().unwrap();
        acknowledge.send(()).unwrap();
        value
    });
    ready.recv_timeout(Duration::from_secs(20)).unwrap();
    let (finished, watching) = std::sync::mpsc::channel();
    let failsafe_release = release.clone();
    let watchdog = std::thread::spawn(move || {
        if watching.recv_timeout(Duration::from_secs(10)).is_err() {
            let _ = failsafe_release.send(());
        }
    });
    let result = join(worker);
    let _ = release.send(());
    let _ = finished.send(());
    done.recv_timeout(Duration::from_secs(20)).unwrap();
    watchdog.join().unwrap();
    result
}

#[test]
fn selftest_worker_joins_refuse_unfinished_and_panicked_workers() {
    let refused = with_blocked_worker((), |worker| {
        join_qemu_worker(worker, Instant::now(), "blocked fixture")
    });
    assert!(refused.unwrap_err().to_string().contains("join deadline"));
    let worker = std::thread::spawn(|| panic!("fixture worker panic"));
    let error = join_qemu_worker(
        worker,
        Instant::now() + Duration::from_secs(20),
        "panic fixture",
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("panicked"));
    let worker = std::thread::spawn(|| 23);
    assert_eq!(
        join_qemu_worker(
            worker,
            Instant::now() + Duration::from_secs(20),
            "complete fixture"
        )
        .unwrap(),
        23
    );
}

#[test]
fn boot_serial_join_refuses_an_unfinished_reader() {
    let result = with_blocked_worker(Ok(BootSerialObservation::default()), |worker| {
        finish_boot_serial(worker, Instant::now())
    });
    let error = result.expect_err("an unfinished boot reader must be refused");
    assert!(error.to_string().contains("join deadline"));
}

#[test]
fn selftest_timeout_classification_survives_serial_and_stop_failures() {
    let timeout = Duration::from_secs(17);
    let exact = "self-test did not finish within 17 seconds";
    let clean = combine_qemu_observation(
        selftest_exit(SelftestTermination::TimedOut, timeout),
        Ok(()),
    );
    assert_eq!(clean.unwrap_err().to_string(), exact);
    let dirty = combine_qemu_observation(
        selftest_exit(SelftestTermination::TimedOut, timeout),
        Err::<(), _>(anyhow!("fixture serial join failure")),
    );
    let dirty = combine_qemu_cleanup(dirty, Err(anyhow!("fixture stop failure")))
        .unwrap_err()
        .to_string();
    assert!(dirty.contains(exact));
    assert!(dirty.contains("fixture serial join failure"));
    assert!(dirty.contains("fixture stop failure"));
    assert_ne!(
        dirty, exact,
        "a cleanup failure must not earn mutation timeout credit"
    );
    assert_eq!(
        selftest_exit(SelftestTermination::Exited(Some(33)), timeout).unwrap(),
        Some(33)
    );
    assert_eq!(
        selftest_exit(SelftestTermination::Panicked, timeout).unwrap(),
        None
    );
    assert!(selftest_exit(SelftestTermination::SerialFailed, timeout).is_err());
}

#[test]
fn qemu_stop_attempts_termination_after_a_poll_failure_and_retains_every_error() {
    struct FailedPoll {
        polls: std::collections::VecDeque<std::result::Result<bool, &'static str>>,
        killed: bool,
        reject_kill: bool,
    }
    fn poll(child: &mut FailedPoll) -> std::io::Result<bool> {
        child
            .polls
            .pop_front()
            .expect("unexpected poll")
            .map_err(std::io::Error::other)
    }
    fn kill(child: &mut FailedPoll) -> std::io::Result<()> {
        child.killed = true;
        if child.reject_kill {
            Err(std::io::Error::other("fixture kill failure"))
        } else {
            Ok(())
        }
    }
    for (reject_kill, after, evidence) in [
        (false, Ok(true), None),
        (
            false,
            Err("fixture reap failure"),
            Some("fixture reap failure"),
        ),
        (
            true,
            Err("fixture recheck failure"),
            Some("fixture recheck failure"),
        ),
        (true, Ok(false), Some("fixture kill failure")),
        (false, Ok(false), Some("did not exit")),
    ] {
        let mut child = FailedPoll {
            polls: [Err("fixture initial poll failure"), after].into(),
            killed: false,
            reject_kill,
        };
        let error = stop_qemu_with(&mut child, poll, kill, Duration::ZERO)
            .expect_err("the initial polling failure must remain visible");
        let text = format!("{error:#}");
        assert!(child.killed, "poll failure skipped termination: {text}");
        assert!(
            child.polls.is_empty(),
            "termination was not followed by a poll"
        );
        assert!(text.contains("fixture initial poll failure"), "{text}");
        if reject_kill {
            assert!(text.contains("fixture kill failure"), "{text}");
        }
        if let Some(evidence) = evidence {
            assert!(text.contains(evidence), "{text}");
        }
    }
}

#[test]
fn selftest_owner_precedes_setup_and_stop_precedes_worker_joins() {
    let source = concat!(
        include_str!("../../qemu.rs"),
        include_str!("../debug.rs"),
        include_str!("../run_files.rs"),
        include_str!("../storage.rs"),
        include_str!("../serial.rs"),
        include_str!("../selftest.rs"),
        include_str!("../boot_check.rs")
    );
    let signature = "fn observe_selftest_run(\n    paths: &Paths,\n    image: &Path,\n    timeout: Duration,\n    echo_serial: bool,\n    marker_order: InteractiveMarkerOrder,\n) -> Result<(SelftestOutcome, Option<PerformanceSample>)> {";
    let body = compact_cpp(&rust_function_body(source, signature));
    let after_spawn = body
        .split_once(".spawn()")
        .unwrap()
        .1
        .split_once("?;")
        .unwrap()
        .1;
    assert!(after_spawn.starts_with("letmutmachine=QemuLifetime::new(child,stop_qemu);"));
    let cancellation = body.find("machine.cancel();").unwrap();
    let stop = body.find("letstopped=machine.stop();").unwrap();
    let close = body.find("serial.close();").unwrap();
    let joins = body
        .find("letjoin_deadline=Instant::now()+QEMU_WORKER_STOP_TIMEOUT;")
        .unwrap();
    assert!(cancellation < stop && stop < close && close < joins);
    assert!(body[joins..].contains("finish_injector(worker,&injection_cancelled,join_deadline)"));
    assert!(body[joins..].contains("finish_selftest_serial(worker,join_deadline)"));
    assert!(body.contains("injector.as_ref().is_some_and(JoinHandle::is_finished)"));
    assert!(body.contains("std::thread::spawn(move||report_hang(qmp.as_mut()))"));
    assert!(body.contains("worker_finished_by(&worker,Instant::now()+MONITOR_TIMEOUT);"));
    assert!(body.contains(
        "letoutcome=observed.and_then(|termination|selftest_exit(termination,timeout));"
    ));
    assert!(body.contains("combine_qemu_observation(outcome,serial_result)"));
    assert!(body.ends_with("combine_qemu_cleanup(outcome,cleanup)"));
    for signature in [
        "pub fn run_boot_check(paths: &Paths, image: &Path, settle: Duration) -> Result<()> {",
        "fn observe_profile(\n    paths: &Paths,\n    image: &Path,\n    settle: Duration,\n    profile: DegradedProfile,\n) -> Result<DegradedOutcome> {",
    ] {
        let body = compact_cpp(&rust_function_body(source, signature));
        let stop = body.find("letstop_outcome=stop_qemu(&mutchild);").unwrap();
        let join = body
            .find("finish_boot_serial(forwarder,Instant::now()+QEMU_WORKER_STOP_TIMEOUT)")
            .or_else(|| {
                body.find("finish_boot_serial(forwarder,Instant::now()+QEMU_WORKER_STOP_TIMEOUT,)")
            })
            .expect("the boot serial reader has the exact bounded join arguments");
        assert!(stop < join);
        assert!(
            body.contains("combine_qemu_cleanup(outcome,stop_outcome)?")
                || body.contains("combine_qemu_observation(outcome,serial_outcome),stop_outcome,")
        );
    }
}

#[test]
fn stopping_the_owner_releases_a_qmp_writer_before_join() {
    struct BlockedWrite {
        entered: std::sync::mpsc::Sender<()>,
        stopped: std::sync::mpsc::Receiver<bool>,
        received_stop: std::sync::mpsc::Sender<bool>,
        completed: std::sync::mpsc::Sender<()>,
    }
    impl std::io::Write for BlockedWrite {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            self.entered.send(()).unwrap();
            let stopped = self.stopped.recv().unwrap();
            self.received_stop.send(stopped).unwrap();
            let error = std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                if stopped {
                    "fixture peer stopped"
                } else {
                    "fixture stop missing"
                },
            );
            self.completed.send(()).unwrap();
            Err(error)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let (entered, writing) = std::sync::mpsc::channel();
    let (stop, stopped) = std::sync::mpsc::channel();
    let (received_stop, stop_observed) = std::sync::mpsc::channel();
    let (completed, write_completed) = std::sync::mpsc::channel();
    let fallback_stop = stop.clone();
    let failsafe_stop = stop.clone();
    let mut owner = QemuLifetime::new(stop, |stop| {
        stop.send(true).context("stopping fixture control peer")
    });
    let cancelled = Arc::clone(&owner.cancelled);
    let worker = std::thread::spawn(move || {
        Qmp::negotiate(
            BlockedWrite {
                entered,
                stopped,
                received_stop,
                completed,
            },
            std::io::Cursor::new(b"{\"QMP\":{}}\n"),
            Instant::now() + Duration::from_secs(20),
            &cancelled,
        )
    });
    writing.recv_timeout(Duration::from_secs(20)).unwrap();
    let (finished, watching) = std::sync::mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        if watching.recv_timeout(Duration::from_secs(10)).is_err() {
            let _ = failsafe_stop.send(false);
        }
    });
    let stopped = owner.stop();
    let result = join_qemu_worker(
        worker,
        Instant::now() + Duration::from_secs(20),
        "blocked QMP writer",
    );
    // A cancellation-aware QMP caller may finish before the writer. Always
    // release the fake peer, even for the missing-stop mutant, then collect
    // independent stop and completion evidence before any assertion.
    let _ = fallback_stop.send(false);
    let stop_seen = stop_observed.recv_timeout(Duration::from_secs(20));
    let completed = write_completed.recv_timeout(Duration::from_secs(20));
    let _ = finished.send(());
    watchdog.join().unwrap();
    stopped.unwrap();
    assert!(
        result.unwrap().is_err(),
        "a stopped writer negotiated successfully"
    );
    completed.expect("the fake control write must finish on every path");
    assert!(
        stop_seen.expect("the fake writer must report its release reason"),
        "owner stop did not release the control writer; only the fixture fail-safe did"
    );
}

#[test]
fn qemu_stop_refuses_timeout_and_a_live_child_after_kill_refusal() {
    let mut stuck = fake_qemu();
    stuck.exit_after_kill_polls = None;
    let error = stop_qemu_with(
        &mut stuck,
        FakeQemuChild::exited,
        FakeQemuChild::terminate,
        Duration::ZERO,
    )
    .unwrap_err();
    assert!(error.to_string().contains("did not exit"));

    let mut refused = fake_qemu();
    refused.reject_kill = true;
    let error = stop_qemu_with(
        &mut refused,
        FakeQemuChild::exited,
        FakeQemuChild::terminate,
        Duration::ZERO,
    )
    .unwrap_err();
    assert!(error.to_string().contains("terminating QEMU"));

    let mut raced = fake_qemu();
    raced.reject_kill = true;
    raced.exit_on_rejected_kill = true;
    stop_qemu_with(
        &mut raced,
        FakeQemuChild::exited,
        FakeQemuChild::terminate,
        Duration::ZERO,
    )
    .unwrap();
}
