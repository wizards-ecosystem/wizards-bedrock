use super::*;

#[test]
fn the_human_wait_fits_the_harness_patience() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root")
        .to_path_buf();
    let input = crate::contracts::module_source(&root, "kernel/src/input.rs");
    let wait_ms: u64 = input
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("pub(crate) const HUMAN_WAIT_MS: u64 = ")
        })
        .and_then(|value| {
            let digits = value.trim_end_matches(';').replace('_', "");
            <u64 as core::str::FromStr>::from_str(&digits).ok()
        })
        .expect("no HUMAN_WAIT_MS in input.rs");

    const ROUNDS: u64 = 3;
    let unanswered = Duration::from_millis(wait_ms * ROUNDS);
    assert!(
        unanswered < crate::tasks::MUTATION_SELFTEST_TIMEOUT,
        "{ROUNDS} rounds of {wait_ms}ms exceed the {:?} mutation timeout",
        crate::tasks::MUTATION_SELFTEST_TIMEOUT
    );
}

#[test]
fn shipped_execution_evidence_requires_a_user_call_and_deferred_once_only_report() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let user = [
        "kernel/src/user.rs",
        "kernel/src/user/call.rs",
        "kernel/src/user/construction.rs",
        "kernel/src/user/load.rs",
        "kernel/src/user/reap.rs",
        "kernel/src/user/selftest.rs",
    ]
    .map(|relative| std::fs::read_to_string(root.join(relative)).unwrap())
    .concat();
    let thread = std::fs::read_to_string(root.join("kernel/src/thread.rs")).unwrap();
    let active = compact_cpp(&rust_without_comments(&user));
    for declaration in [
        "staticEXECUTED:AtomicBool=AtomicBool::new(false);",
        "staticEXECUTION_REPORTED:AtomicBool=AtomicBool::new(false);",
    ] {
        assert!(
            active.contains(declaration),
            "observation begins unearned: {declaration}"
        );
    }
    assert_eq!(active.matches("EXECUTED.store(").count(), 1);
    let call = compact_cpp(&rust_function_body(
        &user,
        "pub(crate) fn call(frame: &TrapFrame) {",
    ));
    assert!(call.starts_with(
        "letslot=task::current();ifframe.code_selector()&0b11==3{EXECUTED.store(true,Ordering::Release);}"
    ));
    assert!(
        !call.contains("info!"),
        "trap reporting must not acquire the console lock"
    );
    let reporter = compact_cpp(&rust_function_body(
        &user,
        "pub(crate) fn report_execution() -> bool {",
    ));
    assert_eq!(
        reporter,
        "if!EXECUTED.load(Ordering::Acquire)||EXECUTION_REPORTED.swap(true,Ordering::AcqRel){returnfalse;}info!();true"
    );
    let idle = compact_cpp(&rust_function_body(&thread, "fn idle_loop() {"));
    assert!(
        idle.contains(
            "ifcrate::user::report_execution()||crate::user::report_refusals(){continue;}"
        )
    );
    assert_eq!(SHIPPED_USER_EXECUTION_MARKER, "user: ring 3 call observed");
    assert_eq!(
        user.matches("info!(\"user: ring 3 call observed\");")
            .count(),
        1
    );
}

#[test]
fn the_exit_codes_are_the_kernels_doubled() {
    fn qemu_exit_code(written: i32) -> i32 {
        (written << 1) | 1
    }

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");
    let debug_exit = std::fs::read_to_string(root.join("kernel/src/arch/x86_64/debug_exit.rs"))
        .expect("debug_exit.rs");

    let value = |name: &str| -> i32 {
        let marked = format!("const {name}: u32 = ");
        let rest = debug_exit
            .split_once(&marked)
            .unwrap_or_else(|| panic!("no `{name}` in debug_exit.rs"))
            .1;
        let literal = rest
            .split_once(';')
            .expect("a terminated constant")
            .0
            .trim()
            .trim_start_matches("0x");
        i32::from_str_radix(literal, 16).expect("a hex literal")
    };

    assert_eq!(
        qemu_exit_code(value("PASSED")),
        SELFTEST_PASSED,
        "the kernel's PASSED does not arrive as the code the harness waits for"
    );
    assert_eq!(
        qemu_exit_code(value("FAILED")),
        SELFTEST_FAILED,
        "the kernel's FAILED does not arrive as the code the harness waits for"
    );
    assert_ne!(SELFTEST_PASSED, SELFTEST_FAILED);
}

#[derive(Debug, PartialEq, Eq)]
struct DevfsEntry {
    variant: String,
    name: Vec<u8>,
    description: String,
}

fn device_variant(path: &syn::Path, owner: &str) -> Result<String> {
    ensure!(
        path.segments.len() == 2
            && path.segments[0].ident == owner
            && path
                .segments
                .iter()
                .all(|segment| segment.arguments.is_empty()),
        "expected {owner}::Variant"
    );
    Ok(path.segments[1].ident.to_string())
}

fn devfs_method<'a>(syntax: &'a syn::File, name: &str) -> Result<&'a syn::ImplItemFn> {
    let methods = syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Impl(item) if item.trait_.is_none() => Some(item),
            _ => None,
        })
        .filter(|item| {
            matches!(
                item.self_ty.as_ref(),
                syn::Type::Path(path)
                    if path.qself.is_none() && path.path.is_ident("Device")
            )
        })
        .flat_map(|item| &item.items)
        .filter_map(|item| match item {
            syn::ImplItem::Fn(method) if method.sig.ident == name => Some(method),
            _ => None,
        })
        .collect::<Vec<_>>();
    ensure!(methods.len() == 1, "expected one Device::{name} method");
    Ok(methods[0])
}

fn devfs_match<T>(
    syntax: &syn::File,
    method: &str,
    literal: impl Fn(&syn::Lit) -> Option<T>,
) -> Result<std::collections::BTreeMap<String, T>> {
    let body = &devfs_method(syntax, method)?.block.stmts;
    let [syn::Stmt::Expr(syn::Expr::Match(matched), None)] = body.as_slice() else {
        bail!("Device::{method} must contain exactly one match expression");
    };
    ensure!(
        matches!(
            matched.expr.as_ref(),
            syn::Expr::Path(path) if path.qself.is_none() && path.path.is_ident("self")
        ),
        "Device::{method} must match self"
    );

    let mut values = std::collections::BTreeMap::new();
    for arm in &matched.arms {
        ensure!(arm.guard.is_none(), "Device::{method} has a guarded arm");
        let syn::Pat::Path(pattern) = &arm.pat else {
            bail!("Device::{method} has a non-variant pattern");
        };
        ensure!(
            pattern.qself.is_none(),
            "Device::{method} has a qualified pattern"
        );
        let variant = device_variant(&pattern.path, "Self")?;
        let syn::Expr::Lit(value) = arm.body.as_ref() else {
            bail!("Device::{method}::{variant} is not a literal");
        };
        let value = literal(&value.lit)
            .with_context(|| format!("Device::{method}::{variant} has the wrong literal kind"))?;
        ensure!(
            values.insert(variant.clone(), value).is_none(),
            "Device::{method} repeats {variant}"
        );
    }
    Ok(values)
}

fn devfs_contract(source: &str) -> Result<Vec<DevfsEntry>> {
    let syntax = syn::parse_file(source).context("parsing kernel/src/devfs.rs")?;
    let enums = syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Enum(item) if item.ident == "Device" => Some(item),
            _ => None,
        })
        .collect::<Vec<_>>();
    ensure!(enums.len() == 1, "expected one Device enum");

    let mut variants = std::collections::BTreeSet::new();
    for variant in &enums[0].variants {
        ensure!(
            matches!(variant.fields, syn::Fields::Unit),
            "Device::{} is not a unit variant",
            variant.ident
        );
        ensure!(
            variants.insert(variant.ident.to_string()),
            "Device repeats {}",
            variant.ident
        );
    }

    let declarations = syntax
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Const(item) if item.ident == "DEVICES" => Some(item),
            _ => None,
        })
        .collect::<Vec<_>>();
    ensure!(declarations.len() == 1, "expected one DEVICES constant");
    let syn::Expr::Array(array) = declarations[0].expr.as_ref() else {
        bail!("DEVICES is not an array literal");
    };
    let mut listed = Vec::new();
    for element in &array.elems {
        let syn::Expr::Path(path) = element else {
            bail!("DEVICES contains a non-variant expression");
        };
        ensure!(
            path.qself.is_none(),
            "DEVICES contains a qualified expression"
        );
        listed.push(device_variant(&path.path, "Device")?);
    }
    let listed_set = listed
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    ensure!(
        listed.len() == listed_set.len(),
        "DEVICES contains a duplicate variant"
    );
    ensure!(
        listed_set == variants,
        "DEVICES does not match Device variants: listed {listed_set:?}, declared {variants:?}"
    );

    let mut names = devfs_match(&syntax, "name", |literal| match literal {
        syn::Lit::ByteStr(value) => Some(value.value()),
        _ => None,
    })?;
    let mut descriptions = devfs_match(&syntax, "describes", |literal| match literal {
        syn::Lit::Str(value) => Some(value.value()),
        _ => None,
    })?;
    ensure!(
        names
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            == variants,
        "Device::name does not match Device variants"
    );
    ensure!(
        descriptions
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            == variants,
        "Device::describes does not match Device variants"
    );

    let mut unique_names = std::collections::BTreeSet::new();
    listed
        .into_iter()
        .map(|variant| {
            let name = names
                .remove(&variant)
                .context("a listed device has no name")?;
            let description = descriptions
                .remove(&variant)
                .context("a listed device has no description")?;
            ensure!(!name.is_empty(), "Device::{variant} has an empty name");
            ensure!(
                unique_names.insert(name.clone()),
                "Device::{variant} repeats a device name"
            );
            ensure!(
                !description.is_empty(),
                "Device::{variant} has an empty description"
            );
            Ok(DevfsEntry {
                variant,
                name,
                description,
            })
        })
        .collect()
}

fn devfs_fixture(variants: &str, listed: &str, names: &str, descriptions: &str) -> String {
    format!(
        "enum Device {{ {variants} }}\n\
             impl Device {{\n\
                 fn name(self) -> &'static [u8] {{ match self {{ {names} }} }}\n\
                 fn describes(self) -> &'static str {{ match self {{ {descriptions} }} }}\n\
             }}\n\
             const DEVICES: [Device; 3] = [{listed}];\n"
    )
}

#[test]
fn a_complete_devfs_inventory_is_accepted() {
    let fixture = devfs_fixture(
        "Storage, Console, Timer",
        "Device::Storage, Device::Console, Device::Timer",
        "Self::Storage => b\"storage\", Self::Console => b\"console\", Self::Timer => b\"timer\",",
        "Self::Storage => \"block\", Self::Console => \"character\", Self::Timer => \"clock\",",
    );
    assert_eq!(
        devfs_contract(&fixture).expect("complete inventory").len(),
        3
    );
}

#[test]
fn a_devfs_variant_omitted_from_the_listing_is_refused() {
    let fixture = devfs_fixture(
        "Storage, Console, Timer",
        "Device::Storage, Device::Console",
        "Self::Storage => b\"storage\", Self::Console => b\"console\", Self::Timer => b\"timer\",",
        "Self::Storage => \"block\", Self::Console => \"character\", Self::Timer => \"clock\",",
    );
    let error = devfs_contract(&fixture).expect_err("Timer is absent from DEVICES");
    assert!(error.to_string().contains("DEVICES does not match"));
}

#[test]
fn a_devfs_variant_omitted_from_either_description_is_refused() {
    for (names, descriptions, expected) in [
        (
            "Self::Storage => b\"storage\", Self::Console => b\"console\",",
            "Self::Storage => \"block\", Self::Console => \"character\", Self::Timer => \"clock\",",
            "Device::name",
        ),
        (
            "Self::Storage => b\"storage\", Self::Console => b\"console\", Self::Timer => b\"timer\",",
            "Self::Storage => \"block\", Self::Console => \"character\",",
            "Device::describes",
        ),
    ] {
        let fixture = devfs_fixture(
            "Storage, Console, Timer",
            "Device::Storage, Device::Console, Device::Timer",
            names,
            descriptions,
        );
        assert!(
            devfs_contract(&fixture)
                .expect_err("a descriptive match omitted Timer")
                .to_string()
                .contains(expected)
        );
    }
}

#[test]
fn duplicate_devfs_authority_is_refused() {
    let duplicate_listing = devfs_fixture(
        "Storage, Console",
        "Device::Storage, Device::Console, Device::Storage",
        "Self::Storage => b\"storage\", Self::Console => b\"console\",",
        "Self::Storage => \"block\", Self::Console => \"character\",",
    );
    assert!(
        devfs_contract(&duplicate_listing)
            .expect_err("duplicate listing")
            .to_string()
            .contains("duplicate variant")
    );

    let duplicate_name = devfs_fixture(
        "Storage, Console, Timer",
        "Device::Storage, Device::Console, Device::Timer",
        "Self::Storage => b\"device\", Self::Console => b\"console\", Self::Timer => b\"device\",",
        "Self::Storage => \"block\", Self::Console => \"character\", Self::Timer => \"clock\",",
    );
    assert!(
        devfs_contract(&duplicate_name)
            .expect_err("duplicate name")
            .to_string()
            .contains("repeats a device name")
    );
}

#[test]
fn the_devfs_source_inventory_matches_every_variant() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");
    let source = std::fs::read_to_string(repository.join("kernel/src/devfs.rs")).expect("devfs.rs");
    devfs_contract(&source).expect("complete devfs source inventory");
}

#[test]
fn the_serial_contract_matches_the_kernel_source() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");
    let root = repository.join("kernel").join("src");
    let selftest = crate::contracts::module_source(&root, "selftest.rs");
    let main = std::fs::read_to_string(root.join("main.rs")).expect("main.rs");
    let devfs = std::fs::read_to_string(root.join("devfs.rs")).expect("devfs.rs");
    let tmpfs = std::fs::read_to_string(root.join("tmpfs.rs")).expect("tmpfs.rs");
    let panic = std::fs::read_to_string(root.join("panic.rs")).expect("panic.rs");
    let user = [
        "user.rs",
        "user/call.rs",
        "user/construction.rs",
        "user/load.rs",
        "user/reap.rs",
        "user/selftest.rs",
    ]
    .map(|relative| std::fs::read_to_string(root.join(relative)).expect(relative))
    .concat();
    let apic = std::fs::read_to_string(repository.join("wost/src/arch/x86_64/apic.rs"))
        .expect("wost apic.rs");
    let cpu = std::fs::read_to_string(repository.join("wost/src/arch/x86_64/cpu.rs"))
        .expect("wost cpu.rs");
    let cpuid = std::fs::read_to_string(repository.join("libs/x86-cpuid/src/lib.rs"))
        .expect("x86-cpuid lib.rs");

    let constant = |source: &str, name: &str| -> String {
        source
            .lines()
            .find_map(|line| line.trim().strip_prefix(&format!("const {name}: usize = ")))
            .map(|value| value.trim_end_matches(';').to_string())
            .unwrap_or_else(|| panic!("no `{name}` in the kernel source"))
    };

    assert_eq!(
        constant(&selftest, "EXPECTED_CHECKS"),
        EXPECTED_SELFTEST_CHECKS.to_string(),
        "the check count drifted"
    );
    assert_eq!(
        format!("v{}", constant(&selftest, "VERDICT_VERSION")),
        SELFTEST_VERDICT_VERSION,
        "the verdict version drifted"
    );
    assert!(
        selftest.contains(&format!("info!(\"{SELFTEST_IMAGE_MARKER}\");")),
        "the optimization-stable self-test image marker drifted"
    );
    assert!(
        selftest.contains("info!(\"selftest: {name}\");")
            && selftest.contains("error!(\"selftest: {name} FAILED\");"),
        "the exact self-test check records drifted from their producer"
    );
    assert!(
        selftest.contains("\"selftest: verdict v{} PASS passed={} failed={} expected={}\"")
            && selftest.contains("\"selftest: verdict v{} FAIL passed={} failed={} expected={}\""),
        "the exact self-test verdict records drifted from their producers"
    );
    assert!(
        selftest.contains(
            "\"selftest: baseline v1 boot_ns={} frames_free={} heap_slab={} heap_large={} heap_live={} handles_committed={} handles_live={} trap_ticks={} storage_bytes={} storage_ticks={} counter_hz={}\""
        ),
        "the exact self-test baseline record drifted from its producer"
    );
    assert!(
        selftest.contains("fn awaiting(marker: &str, press: &str) {\n    info!(\"{marker}\");")
            && selftest
                .contains("awaiting(\n        \"selftest: awaiting storage-delay keystroke\",")
            && selftest.contains("awaiting(\n        \"selftest: awaiting injected keystroke\",")
            && selftest.contains("awaiting(\n        \"selftest: awaiting translated keystroke\","),
        "the exact interactive info records drifted from their producer"
    );
    assert!(
        selftest.contains(
            "let woke_through_polling = runs_until(1_000, || {\n        READER_SAW.load(Ordering::Acquire) != 0 && input::poller_passes() > passes_before\n    });"
        ),
        "the polled-reader witness does not await both consequences"
    );
    assert!(
        main.contains("\"kernel: reading the keyboard, uptime {} ms\"")
            && SETTLED_MARKER == "kernel: reading the keyboard, uptime ",
        "the exact steady-state record drifted from its producer"
    );
    assert!(
        main.contains(
            "\"cpu: user isolation paired roots: forced for self-test; shared proof retained: {reason}\""
        )
            && main.contains("info!(\"cpu: user isolation shared kernel mappings: {reason}\")")
            && cpuid.contains("SharedKernelMappingReasonKind::QemuTcg => \"qemu_tcg\"")
            && SELFTEST_QEMU_TCG_ISOLATION_MARKER
                == "cpu: user isolation paired roots: forced for self-test; shared proof retained: qemu_tcg"
            && SHIPPED_QEMU_TCG_ISOLATION_MARKER
                == "cpu: user isolation shared kernel mappings: qemu_tcg",
        "the exact QEMU TCG evidence drifted from its producers"
    );
    assert!(
        main.contains("info!(\"nvme: candidate {ordinal} retired cleanly; trying next\")")
            && NVME_FALLBACK_MARKER == "nvme: candidate 1 retired cleanly; trying next",
        "the exact first-candidate retirement record drifted from its producer"
    );
    assert!(
        main.contains("warn!(\"storage: no DMA isolation\")")
            && NO_ISOLATION_MARKER == "storage: no DMA isolation"
            && main.contains("StorageSelection::Seeking => info!(\"nvme: no usable controller\")")
            && NO_USABLE_CONTROLLER_MARKER == "nvme: no usable controller",
        "the exact unisolated refusal records drifted from their producers"
    );
    assert_eq!(
        UNISOLATED_REQUIRED_MARKERS.len(),
        5,
        "the unisolated boot requires the strict fence, TCG policy, refusal, clean retirement and empty selection"
    );
    let qemu = concat!(
        include_str!("../../qemu.rs"),
        include_str!("../debug.rs"),
        include_str!("../run_files.rs"),
        include_str!("../storage.rs"),
        include_str!("../serial.rs"),
        include_str!("../selftest.rs"),
        include_str!("../boot_check.rs")
    );
    let storage = qemu
        .split_once("fn storage(root: &Path)")
        .expect("the QEMU storage profile")
        .1
        .split_once("const SELFTEST_PASSED")
        .expect("the end of the QEMU storage profile")
        .0;
    assert!(
        storage.find("nvme,serial=wizard-empty")
            < storage.find("nvme,serial=wizard0,drive=wizardnvm")
            && main.contains(
                "storage += 1;\n            if matches!(selection, StorageSelection::Seeking) {\n                selection = try_controller(function.at, storage);"
            ),
        "QEMU's deliberately empty controller is no longer candidate 1"
    );
    assert!(
        main.contains("warn!(\"pci: the configuration ports are already claimed; no bus\")"),
        "the exact no-PCI warning drifted from its producer"
    );
    let no_x2apic_reason = NO_X2APIC_DIAGNOSIS
        .strip_prefix("local apic: ")
        .and_then(|message| message.strip_suffix("; interrupts stay off"))
        .expect("the no-x2APIC diagnosis has the canonical envelope");
    assert!(
        main.contains("warn!(\"local apic: {reason}; interrupts stay off\")")
            && apic.contains(&format!(
                "Self::NoExtendedMode => core::fmt::Write::write_str(\n                f,\n                \"{no_x2apic_reason}\",\n            ),"
            )),
        "the exact no-x2APIC diagnosis drifted from its two producers"
    );
    assert!(
        main.contains("warn!(\"cpu: extended state unavailable; ring 3 stays closed\")"),
        "the exact extended-state warning drifted from its producer"
    );
    let user_reason = DEGRADED_USER_MARKER
        .strip_prefix("user: no program: ")
        .expect("the degraded user marker has the boot prefix");
    assert!(
        main.contains("Err(reason) => warn!(\"user: no program: {reason}\")")
            && user.contains(user_reason),
        "the kernel does not compose {DEGRADED_USER_MARKER:?}"
    );
    assert_eq!(
        DEGRADED_FILESYSTEM_MARKER,
        "tmpfs: 1 file(s) and 1 directory(ies) at the root"
    );
    assert!(
        main.contains(
            "info!(\"tmpfs: {files} file(s) and {directories} directory(ies) at the root\")"
        ) && tmpfs.contains(
            "add(\n        &mut arena,\n        ROOT,\n        b\"MOTD\",\n        Some("
        ) && tmpfs.contains("let notes = add(&mut arena, ROOT, b\"NOTES\", None);")
            && tmpfs.matches("        ROOT,\n").count() == 1
            && tmpfs.matches("add(&mut arena, ROOT,").count() == 1,
        "the exact degraded tmpfs success record drifted from its producers"
    );
    let devices = devfs_contract(&devfs).expect("complete devfs source inventory");
    let storage = devices
        .iter()
        .find(|device| device.variant == "Storage")
        .expect("a Storage device");
    let console = devices
        .iter()
        .find(|device| device.variant == "Console")
        .expect("a Console device");
    let storage_record = format!(
        "devfs: {} is {}",
        core::str::from_utf8(&storage.name).expect("an ASCII storage name"),
        storage.description
    );
    let console_record = format!(
        "devfs: {} is {}",
        core::str::from_utf8(&console.name).expect("an ASCII console name"),
        console.description
    );
    assert!(
        main.contains("Ok(vfs::Node::Device(found)) => info!(")
            && main.contains("\"devfs: {} is {}\"")
            && storage_record == DEGRADED_STORAGE_MARKER
            && console_record == DEGRADED_CONSOLE_MARKER,
        "the devfs degraded markers no longer compose from the kernel sources"
    );
    assert!(
        main.contains("info!(\"cpu: protections {protections}\")")
            && cpu.contains(
                "(self.write_protect, \"wp\"),\n            (self.smep, \"smep\"),\n            (self.smap, \"smap\"),\n            (self.umip, \"umip\"),\n            (self.nx, \"nx\"),"
            )
            && DEGRADED_PROTECTIONS_MARKER == "cpu: protections wp nx",
        "the degraded protection record drifted from its ordered producer"
    );
    let watcher = qemu
        .split_once("fn watch_serial(")
        .expect("the serial watcher")
        .1
        .split_once("fn finish_boot_serial(")
        .expect("the end of the serial watcher")
        .0;
    assert!(
        watcher.contains("if is_settled_record(&text)")
            && watcher.contains("is_required_boot_marker(&text, *record)")
            && !watcher.contains("text.contains(SETTLED_MARKER)"),
        "the serial watcher bypasses exact boot-record matching"
    );
    let forwarder = qemu
        .split_once("fn forward_serial(")
        .expect("the self-test serial forwarder")
        .1
        .split_once("fn finish_selftest_serial(")
        .expect("the end of the self-test serial forwarder")
        .0;
    assert!(
        forwarder.contains("match observe_interactive_marker(")
            && forwarder.contains("&mut next_interactive_marker,")
            && forwarder.contains("marker_order,")
            && forwarder.contains("markers_seen.store(next_interactive_marker, Ordering::Release)")
            && !forwarder.contains("text.contains(STORAGE_DELAY_MARKER)")
            && !forwarder.contains("text.contains(READY_MARKER)")
            && !forwarder.contains("text.contains(TRANSLATED_MARKER)"),
        "the self-test forwarder bypasses ordered exact interactive records"
    );
    assert!(
        panic.contains("use panic_record::write_record;")
            && panic.contains("let _ = write_record(uart, thread, info);"),
        "the kernel does not consume the shared panic-record contract"
    );
}

#[test]
fn the_uart_reports_every_refused_byte_to_its_formatter() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");
    let uart = std::fs::read_to_string(root.join("kernel/src/arch/x86_64/uart.rs"))
        .expect("kernel uart.rs");

    assert!(
        uart.contains(
            "fn write_byte(&mut self, byte: u8) -> fmt::Result {\n        if !self.wait_for(line_status::TRANSMIT_HOLDING_EMPTY) {\n            return Err(fmt::Error);\n        }\n        PortBlock::write(&mut self.registers, register::DATA, byte);\n        Ok(())\n    }"
        ),
        "the UART can report success after refusing a byte"
    );
    assert!(
        uart.contains("self.write_byte(b'\\r')?;") && uart.contains("self.write_byte(byte)?;"),
        "the UART formatter discards a byte-write refusal"
    );
}

#[test]
fn the_namespaces_signature_is_the_one_the_kernel_looks_for() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");
    let selftest = std::fs::read_to_string(root.join("kernel/src/selftest/storage.rs"))
        .expect("selftest/storage.rs");

    let kernel = selftest
        .lines()
        .find_map(|line| line.trim().strip_prefix("0x5749_5A44_0000_0000 | "))
        .expect("no disk signature in selftest.rs");
    assert_eq!(
        kernel, "lba",
        "the kernel's signature is not the base value ored with the block"
    );

    assert_eq!(super::nvme_signature(0), 0x5749_5A44_0000_0000);
    for lba in [0u64, 1, 3, 7, super::NVME_SIGNED_BLOCKS - 1] {
        assert_eq!(
            super::nvme_signature(lba),
            0x5749_5A44_0000_0000 | lba,
            "block {lba}"
        );
    }
    // Distinct signatures expose an LBA ignored by the reader.
    for lba in 0..super::NVME_SIGNED_BLOCKS {
        for earlier in 0..lba {
            assert_ne!(super::nvme_signature(lba), super::nvme_signature(earlier));
        }
    }
}

#[test]
fn nvme_cleanup_and_bus_master_authority_precede_the_enable_write() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");
    let source = rust_without_comments(
        &[
            "kernel/src/nvme/io.rs",
            "kernel/src/nvme/preparation.rs",
            "kernel/src/nvme/queues.rs",
            "kernel/src/nvme/selftest.rs",
            "kernel/src/nvme.rs",
        ]
        .map(|relative| std::fs::read_to_string(root.join(relative)).expect("kernel nvme source"))
        .concat(),
    );
    let finish = source
        .split_once("fn finish_preparation(")
        .and_then(|(_, source)| source.split_once("fn retire_disabled("))
        .map(|(source, _)| source)
        .expect("no bounded controller-preparation completion body");
    let finish_owner = finish
        .find("let controller = Controller {")
        .expect("no cleanup owner in controller-preparation completion");
    assert!(
        !finish[..finish_owner].contains("write("),
        "controller preparation writes hardware before cleanup owns every published queue"
    );
    let preparation = source
        .find("let preparation = ControllerPreparation::new(registers);")
        .expect("no preparation owner for the mapped controller");
    let owner = source
        .find("let controller = Controller {")
        .expect("no cleanup-owning controller construction");
    let prepared = source
        .find("PrepareOutcome::Prepared(DisabledController {\n        controller,")
        .expect("the cleanup owner is not retained by the disabled controller");
    let admission = source
        .find("match device.admit_bus_master(authority, domain, interrupts) {")
        .expect("no verified bus-master transition");
    let mint = source
        .find("Ok(()) => BusMasterAdmission::Admitted(BusMasterAdmittedController(self))")
        .expect("successful bus-master admission does not mint start authority");
    let start = source
        .find("pub(crate) fn start(")
        .expect("no typed controller start door");
    let consume = source[start..]
        .find("let BusMasterAdmittedController(DisabledController {")
        .map(|offset| start + offset)
        .expect("controller start does not consume admitted ownership");
    let enable = source[start..]
        .find("write(registers, register::CC, configuration)")
        .map(|offset| start + offset)
        .expect("no controller enable write through the cleanup owner");

    assert!(
        preparation < owner
            && owner < prepared
            && prepared < start
            && start < consume
            && consume < enable,
        "controller enable is not ordered after typed cleanup ownership"
    );
    assert!(
        source
            .contains("pub(crate) fn admit_bus_master(\n        mut self,\n        device: &mut crate::pci::Device,")
            && source.contains(") -> BusMasterAdmission")
            && source
                .matches(
                    "Ok(()) => BusMasterAdmission::Admitted(BusMasterAdmittedController(self))"
                )
                .count()
                == 1
            && admission < mint
            && source[start..].contains("prepared: BusMasterAdmittedController")
            && !source[start..consume].contains("prepared: DisabledController"),
        "controller start can bypass consumed bus-master admission authority"
    );
    assert!(
        source
            .matches("write(registers, register::CC, configuration)")
            .count()
            == 1,
        "controller enable is not confined to the cleanup-owning start path"
    );
}

#[test]
fn the_corpus_the_kernel_looks_for_is_the_one_that_is_written() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");
    let selftest = std::fs::read_to_string(root.join("kernel/src/selftest/files.rs"))
        .expect("selftest/files.rs");
    let fs = std::fs::read_to_string(root.join("kernel/src/fs.rs")).expect("fs.rs");
    use crate::image::corpus;

    let kernel_base = fs
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("pub(crate) const VOLUME_SECTOR: u64 = ")
        })
        .map(|value| value.trim_end_matches(';').to_string())
        .expect("no VOLUME_SECTOR in fs.rs");
    assert_eq!(kernel_base, super::NVME_VOLUME_SECTOR.to_string());

    for (constant, expected) in [
        ("SMALL_NAME", corpus::SMALL_NAME.to_string()),
        ("LARGE_NAME", corpus::LARGE_NAME.to_string()),
        ("DIRECTORY_NAME", corpus::DIRECTORY_NAME.to_string()),
        ("NESTED_NAME", corpus::NESTED_NAME.to_string()),
    ] {
        let declared = selftest
            .lines()
            .find_map(|line| {
                line.trim()
                    .trim_start_matches("pub(super) ")
                    .strip_prefix(&format!("const {constant}: &[u8] = b\""))
            })
            .and_then(|rest| rest.split_once('\"').map(|(value, _)| value.to_string()))
            .unwrap_or_else(|| panic!("no {constant} in selftest/files.rs"));
        assert_eq!(declared, expected, "{constant} drifted");
    }

    assert!(
        selftest.contains(&format!(
            "const LARGE_BYTES: usize = {} * 1024;",
            corpus::LARGE_BYTES / 1024
        )),
        "LARGE_BYTES drifted from the corpus"
    );
    assert!(
        selftest.contains("((index as u32).wrapping_mul(2_654_435_761) >> 13) as u8"),
        "the kernel fills the large file differently from xtask"
    );
    // Compare the independent literal with the corpus implementation too.
    for index in [0usize, 1, 511, 4096, corpus::LARGE_BYTES - 1] {
        assert_eq!(
            corpus::large_byte(index),
            ((index as u32).wrapping_mul(2_654_435_761) >> 13) as u8
        );
    }
}

#[test]
fn the_expected_count_is_the_number_of_checks_registered() {
    let selftest = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root")
            .join("kernel")
            .join("src")
            .join("selftest.rs"),
    )
    .expect("selftest.rs");

    let registered = selftest.matches("report.check(").count();
    assert_eq!(
        registered, EXPECTED_SELFTEST_CHECKS,
        "the kernel registers {registered} checks and declares {EXPECTED_SELFTEST_CHECKS}"
    );
}
