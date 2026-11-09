use super::*;

fn validated_entry_fixture_source() -> &'static str {
    r#"pub struct BootInfo;

pub mod limine_glue {
    pub struct Request;
    impl Request { pub const fn new() -> Self { Self } }
    pub struct BaseRevision;
    pub type RequestsStartMarker = Request;
    pub type FramebufferRequest = Request;
    pub type MemoryMapRequest = Request;
    pub type HhdmRequest = Request;
    pub type ExecutableAddressRequest = Request;
    pub type RsdpRequest = Request;
    pub type ModuleRequest = Request;
    pub type RequestsEndMarker = Request;
    pub struct Requests {
        pub base_revision: &'static BaseRevision,
        pub framebuffer: &'static FramebufferRequest,
        pub memory_map: &'static MemoryMapRequest,
        pub hhdm: &'static HhdmRequest,
        pub image: &'static ExecutableAddressRequest,
        pub rsdp: &'static RsdpRequest,
        pub modules: &'static ModuleRequest,
    }
    pub const fn base_revision() -> BaseRevision { BaseRevision }
    pub fn enter(_: Requests, handler: fn(super::BootInfo) -> !) -> ! {
        handler(super::BootInfo)
    }
}

pub mod __macro {
    pub use super::limine_glue::{
        BaseRevision, ExecutableAddressRequest, FramebufferRequest, HhdmRequest,
        MemoryMapRequest, ModuleRequest, Requests, RequestsEndMarker,
        RequestsStartMarker, RsdpRequest, base_revision,
    };
}

pub use limine_glue::enter;

#[macro_export]
macro_rules! entry {
    ($path:path) => {
        const __WOST_ENTRY_SIGNATURE: fn($crate::boot::BootInfo) -> ! = $path;

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests_start_marker")]
        static __WOST_REQUESTS_START: $crate::boot::__macro::RequestsStartMarker =
            $crate::boot::__macro::RequestsStartMarker::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_BASE_REVISION: $crate::boot::__macro::BaseRevision =
            $crate::boot::__macro::base_revision();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_FRAMEBUFFER: $crate::boot::__macro::FramebufferRequest =
            $crate::boot::__macro::FramebufferRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_MEMORY_MAP: $crate::boot::__macro::MemoryMapRequest =
            $crate::boot::__macro::MemoryMapRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_DIRECT_MAP: $crate::boot::__macro::HhdmRequest =
            $crate::boot::__macro::HhdmRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_IMAGE: $crate::boot::__macro::ExecutableAddressRequest =
            $crate::boot::__macro::ExecutableAddressRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_RSDP: $crate::boot::__macro::RsdpRequest =
            $crate::boot::__macro::RsdpRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_MODULES: $crate::boot::__macro::ModuleRequest =
            $crate::boot::__macro::ModuleRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests_end_marker")]
        static __WOST_REQUESTS_END: $crate::boot::__macro::RequestsEndMarker =
            $crate::boot::__macro::RequestsEndMarker::new();

        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        extern "C" fn kmain() -> ! {
            $crate::boot::enter(
                $crate::boot::__macro::Requests {
                    base_revision: &__WOST_BASE_REVISION,
                    framebuffer: &__WOST_FRAMEBUFFER,
                    memory_map: &__WOST_MEMORY_MAP,
                    hhdm: &__WOST_DIRECT_MAP,
                    image: &__WOST_IMAGE,
                    rsdp: &__WOST_RSDP,
                    modules: &__WOST_MODULES,
                },
                __WOST_ENTRY_SIGNATURE,
            )
        }
    };
}
"#
}

#[test]
fn cross_crate_entry_diagnostics_require_the_exact_compiler_origin_bridge() {
    let root = crate::test_support::reserve_temp_directory("wizard-api-entry-origin");
    std::fs::create_dir_all(root.join("libs")).expect("empty entry fixture libs directory");
    std::fs::create_dir_all(root.join("wost/src/boot"))
        .expect("entry fixture wost source directory");
    std::fs::create_dir_all(root.join("kernel/src"))
        .expect("entry fixture kernel source directory");
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"wost\", \"kernel\"]\nresolver = \"3\"\n",
    )
    .expect("entry fixture workspace manifest");
    std::fs::write(
        root.join("wost/Cargo.toml"),
        "[package]\nname = \"wost\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("entry fixture wost manifest");
    std::fs::write(
        root.join("kernel/Cargo.toml"),
        "[package]\nname = \"wizard-kernel\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nwost = { path = \"../wost\" }\n",
    )
    .expect("entry fixture kernel manifest");
    std::fs::write(root.join("wost/src/lib.rs"), "pub mod boot;\n")
        .expect("entry fixture wost root");
    std::fs::write(
        root.join("wost/src/boot.rs"),
        validated_entry_fixture_source(),
    )
    .expect("entry fixture macro source");
    std::fs::write(
        root.join("kernel/src/main.rs"),
        "::wost::entry!(kernel_main);\nmod other;\nfn kernel_main(_: wost::boot::BootInfo) -> ! { loop {} }\nfn main() {}\n",
    )
    .expect("entry fixture kernel source");
    std::fs::write(root.join("kernel/src/other.rs"), "pub fn dormant() {}\n")
        .expect("entry fixture secondary kernel source");

    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root");
    let compiler = compiler_identity(repository).expect("adjacent compiler identity");
    let metadata = isolated_test_cargo_command(&root, &compiler)
        .args(["metadata", "--format-version=1", "--offline"])
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .output()
        .expect("entry fixture Cargo metadata");
    assert!(
        metadata.status.success(),
        "{}{}",
        String::from_utf8_lossy(&metadata.stdout),
        String::from_utf8_lossy(&metadata.stderr)
    );
    let catalog = CargoCatalog::parse(
        &root,
        &root,
        &metadata.stdout,
        &["wost", crate::paths::KERNEL_PACKAGE],
        true,
    )
    .expect("entry fixture Cargo catalog");
    let targets = catalog.targets().cloned().collect::<Vec<_>>();
    let untrusted_inventory = Inventory::discover_with_targets(&root, &targets)
        .expect("entry fixture inventory without authority");
    assert!(untrusted_inventory.entry.is_none());
    let inventory = Inventory::discover_from_catalog(&root, &catalog)
        .expect("entry fixture catalog-authorized inventory");
    let entry = inventory.entry.expect("entry fixture link");
    let kernel_origin = &inventory.files[entry.file].origin;
    let declaration = &inventory.nodes[entry.macro_declaration];
    let contracts = [
        "boot::BootInfo",
        "boot::limine_glue::base_revision",
        "boot::limine_glue::enter",
        "boot::limine_glue::Requests",
    ]
    .map(|symbol| {
        let id = inventory
            .nodes
            .iter()
            .find(|node| node.symbol == symbol)
            .unwrap_or_else(|| panic!("entry fixture has no `{symbol}` node"))
            .id;
        assert!(
            entry
                .contract_nodes
                .iter()
                .any(|contract| contract.id == id)
        );
        (symbol, id)
    });
    let mut enter_evidence = None;
    for (symbol, id) in contracts {
        let instrumentation = (0..inventory.antichain_rounds)
            .find_map(|round| {
                let instrumentation = inventory
                    .instrument(&root, round)
                    .expect("instrument entry fixture");
                instrumentation.ids.contains(&id).then_some(instrumentation)
            })
            .unwrap_or_else(|| panic!("entry fixture has no `{symbol}` instrumentation round"));
        let compiled = isolated_test_cargo_command(&root, &compiler)
            .env("CARGO_TARGET_DIR", root.join("target"))
            .env("CARGO_ENCODED_RUSTFLAGS", "")
            .env("CARGO_BUILD_WARNINGS", "warn")
            .args([
                "check",
                "--offline",
                "--locked",
                "--message-format=json",
                "--package",
                crate::paths::KERNEL_PACKAGE,
                "--bin",
                crate::paths::KERNEL_PACKAGE,
                "--color=never",
            ])
            .arg("--manifest-path")
            .arg(root.join("Cargo.toml"))
            .output()
            .expect("compile entry fixture");
        assert!(
            compiled.status.success(),
            "{symbol}: {}{}",
            String::from_utf8_lossy(&compiled.stdout),
            String::from_utf8_lossy(&compiled.stderr)
        );
        let diagnostics = cargo_compilation_output(&compiled.stdout, None)
            .expect("structured entry fixture diagnostics");
        let crosses = diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.id == id
                    && diagnostic.origin.package_id == kernel_origin.package_id
                    && inventory.file_index(&root, &diagnostic.reported) == Some(declaration.file)
            })
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            !crosses.is_empty(),
            "entry fixture emitted no kernel-origin `{symbol}` definition diagnostics"
        );
        for cross in &crosses {
            let primary_file = inventory
                .file_index(&root, &cross.reported)
                .expect("classified entry definition span");
            let primary_at =
                corrected_marker_location(primary_file, cross.line, cross.column, &instrumentation)
                    .expect("corrected entry definition span");
            assert_eq!(primary_file, declaration.file);
            assert!(declaration.range.contains(primary_at));
            assert!(
                entry
                    .contract_nodes
                    .iter()
                    .find(|contract| contract.id == id)
                    .is_some_and(|contract| contract.primary.contains(primary_at)),
                "`{symbol}` diagnostic escaped its validated macro use site"
            );
            let expansion = cross
                .immediate_expansion
                .as_ref()
                .expect("entry diagnostic immediate expansion");
            assert!(expansion.terminal);
            assert_eq!(expansion.macro_decl_name, "::wost::entry!");
            let expansion_file = inventory
                .file_index(&root, &expansion.location.reported)
                .expect("classified entry invocation span");
            let expansion_at = corrected_marker_location(
                expansion_file,
                expansion.location.line,
                expansion.location.column,
                &instrumentation,
            )
            .expect("corrected entry invocation span");
            assert_eq!(expansion_file, entry.file);
            assert_eq!(expansion_at, entry.invocation.start);
            assert!(
                !cargo_diagnostic_origin_matches(cross, &inventory, &root, primary_file)
                    .expect("entry primary origin comparison")
            );
            assert!(
                cargo_diagnostic_origin_matches(cross, &inventory, &root, expansion_file)
                    .expect("entry expansion origin comparison")
            );

            let no_authority = Uses::default()
                .observe_cargo(
                    RootKind::Production,
                    &untrusted_inventory,
                    &root,
                    std::slice::from_ref(cross),
                    &instrumentation,
                )
                .expect_err("raw target discovery must not carry entry authority");
            assert!(
                format!("{no_authority:#}").contains("Cargo origin"),
                "{symbol}: {no_authority:#}"
            );
        }

        let mut uses = Uses::default();
        uses.observe_cargo(
            RootKind::Production,
            &inventory,
            &root,
            &crosses,
            &instrumentation,
        )
        .expect("exact entry origin bridge");
        assert!(
            uses.edges
                .get(&entry.invocation_node)
                .is_some_and(|used| used.contains(&id))
        );
        assert!(uses.live(&inventory).contains(&id));
        if symbol == "boot::limine_glue::enter" {
            let [cross] = crosses.as_slice() else {
                panic!("entry fixture enter emitted {} diagnostics", crosses.len());
            };
            enter_evidence = Some((cross.clone(), instrumentation));
        }
    }
    let (cross, instrumentation) = enter_evidence.expect("entry fixture enter evidence");
    let enter = inventory
        .nodes
        .iter()
        .find(|node| node.symbol == "boot::limine_glue::enter")
        .expect("entry fixture enter node")
        .id;

    let refuse = |label: &str, diagnostic: MarkerDiagnostic, evidence: &Instrumentation| {
        let error = Uses::default()
            .observe_cargo(
                RootKind::Production,
                &inventory,
                &root,
                &[diagnostic],
                evidence,
            )
            .expect_err(label);
        assert!(
            format!("{error:#}").contains("Cargo origin"),
            "{label}: {error:#}"
        );
    };

    let mut missing = cross.clone();
    missing.immediate_expansion = None;
    refuse("missing entry expansion", missing, &instrumentation);

    let mut nonterminal = cross.clone();
    nonterminal
        .immediate_expansion
        .as_mut()
        .expect("entry expansion")
        .terminal = false;
    refuse("nonterminal entry expansion", nonterminal, &instrumentation);

    let mut wrong_macro = cross.clone();
    wrong_macro
        .immediate_expansion
        .as_mut()
        .expect("entry expansion")
        .macro_decl_name = "evil::entry!".to_string();
    refuse("wrong entry macro identity", wrong_macro, &instrumentation);

    let mut wrong_callsite = cross.clone();
    wrong_callsite
        .immediate_expansion
        .as_mut()
        .expect("entry expansion")
        .location
        .line += 1;
    refuse(
        "wrong entry expansion callsite",
        wrong_callsite,
        &instrumentation,
    );

    let mut wrong_expansion_file = cross.clone();
    wrong_expansion_file
        .immediate_expansion
        .as_mut()
        .expect("entry expansion")
        .location
        .reported = root.join("kernel/src/other.rs").display().to_string();
    refuse(
        "wrong entry expansion source",
        wrong_expansion_file,
        &instrumentation,
    );

    let mut wrong_primary = cross.clone();
    wrong_primary.line = 1;
    wrong_primary.column = 1;
    refuse(
        "entry primary outside its declaration",
        wrong_primary,
        &instrumentation,
    );

    let other_site = entry
        .contract_nodes
        .iter()
        .find(|contract| contract.id != enter)
        .expect("another validated entry-contract site")
        .primary
        .start;
    let inserted_bytes = instrumentation
        .insertions
        .get(&declaration.file)
        .into_iter()
        .flatten()
        .filter(|insertion| {
            insertion.line == other_site.line && insertion.column <= other_site.column
        })
        .map(|insertion| insertion.bytes)
        .sum::<usize>();
    let mut wrong_contract_site = cross.clone();
    wrong_contract_site.line = other_site.line;
    wrong_contract_site.column = other_site.column + inserted_bytes + 1;
    refuse(
        "entry contract identity at another validated site",
        wrong_contract_site,
        &instrumentation,
    );

    let mut non_contract = cross.clone();
    non_contract.id = entry.function_node;
    let mut non_contract_ids = instrumentation.ids.clone();
    non_contract_ids.insert(entry.function_node);
    let non_contract_instrumentation = Instrumentation {
        insertions: instrumentation.insertions.clone(),
        ids: non_contract_ids,
    };
    refuse(
        "non-contract entry diagnostic",
        non_contract,
        &non_contract_instrumentation,
    );

    let wrong_origins = [
        ("wrong entry package", {
            let mut diagnostic = cross.clone();
            diagnostic.origin.package_id = "wrong-package-id".to_string();
            diagnostic
        }),
        ("wrong entry target name", {
            let mut diagnostic = cross.clone();
            diagnostic.origin.target_name = "wrong-target".to_string();
            diagnostic
        }),
        ("wrong entry target kind", {
            let mut diagnostic = cross.clone();
            diagnostic.origin.target_kind = CargoTargetKind::Library;
            diagnostic
        }),
        ("wrong entry target source", {
            let mut diagnostic = cross.clone();
            diagnostic.origin.target_source = root.join("wost/src/lib.rs");
            diagnostic
        }),
    ];
    for (label, diagnostic) in wrong_origins {
        refuse(label, diagnostic, &instrumentation);
    }

    let wost_origin = &inventory.files[declaration.file].origin;
    let mut local = cross;
    local.origin = CargoDiagnosticOrigin {
        package_id: wost_origin.package_id.clone(),
        target_name: wost_origin.name.clone(),
        target_kind: wost_origin.kind,
        target_source: root.join(&wost_origin.relative),
    };
    let mut local_uses = Uses::default();
    local_uses
        .observe_cargo(
            RootKind::Production,
            &inventory,
            &root,
            &[local],
            &instrumentation,
        )
        .expect("same-wost macro definition diagnostic is ordinary opaque evidence");
    assert!(local_uses.edges.is_empty());
    assert!(local_uses.roots.is_empty());
    assert!(
        local_uses
            .dead(&inventory)
            .iter()
            .any(|node| node.id == enter)
    );

    std::fs::remove_dir_all(root).expect("remove entry origin fixture");
}

#[test]
fn wost_entry_contract_binds_the_handler_to_the_linker_entry() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root");
    validate_wost_entry_contract(repository).expect("current entry contract");

    let root = fixture("fn main() {}\n");
    std::fs::create_dir_all(root.join("wost/src/boot")).expect("entry fixture directory");
    let source = include_str!("../../../../wost/src/boot.rs");
    let with_decoy = source.replacen(
        "#[macro_export]\nmacro_rules! entry {",
        "macro_rules! unrelated { () => {}; }\n\n#[macro_export]\nmacro_rules! entry {",
        1,
    );
    assert_ne!(
        source, with_decoy,
        "unrelated macro insertion did not apply"
    );
    std::fs::write(root.join("wost/src/boot.rs"), with_decoy)
        .expect("entry fixture with unrelated macro");
    validate_wost_entry_contract(&root).expect("an unrelated macro is not the entry contract");

    let reject = |find: &str, replace: &str, failure: &str, expected: &str| {
        let changed = source.replacen(find, replace, 1);
        assert_ne!(source, changed, "{failure}: mutation did not apply");
        std::fs::write(root.join("wost/src/boot.rs"), changed)
            .expect("mutated entry-contract fixture");
        let error = validate_wost_entry_contract(&root).expect_err(failure);
        let report = format!("{error:#}");
        assert!(report.contains(expected), "{report}");
    };
    reject(
        "        const __WOST_ENTRY_SIGNATURE:",
        "        pub const __WOST_ENTRY_SIGNATURE:",
        "a visible entry-signature constant must be refused",
        "unapproved or visible",
    );
    reject(
        "        static __WOST_REQUESTS_START:",
        "        pub static __WOST_REQUESTS_START:",
        "a visible request static must be refused",
        "unapproved or visible",
    );
    reject(
        "        extern \"C\" fn kmain() -> ! {",
        "        pub extern \"C\" fn kmain() -> ! {",
        "a visible linker entry must be refused",
        "unapproved or visible",
    );
    reject(
        "#[unsafe(no_mangle)]",
        "#[unsafe(export_name = \"different_entry\")]",
        "a renamed linker entry must be refused",
        "linker-visible",
    );
    reject(
        "        extern \"C\" fn kmain() -> ! {",
        "        pub fn hidden_api() {}\n\n        extern \"C\" fn kmain() -> ! {",
        "an extra generated item must be refused",
        "unapproved",
    );
    reject(
        "$crate::boot::__macro::base_revision();",
        "$crate::boot::__macro::BaseRevision::new();",
        "an indirect base-revision constructor must be refused",
        "base revision",
    );
    reject(
        "framebuffer: &__WOST_FRAMEBUFFER,",
        "framebuffer: &__WOST_MEMORY_MAP,",
        "a miswired request field must be refused",
        "exact request block",
    );
    reject(
        "        extern \"C\" fn kmain() -> ! {\n            $crate::boot::enter(",
        "        extern \"C\" fn kmain() -> ! {\n            let _unapproved_runtime_work = 1;\n            $crate::boot::enter(",
        "additional opaque runtime work must be refused",
        "outside its exact audited contract",
    );
    std::fs::remove_dir_all(root).expect("remove entry-contract fixture");
}

#[test]
fn exact_kernel_entry_reaches_its_handler_without_trusting_other_macro_tokens() {
    let root = fixture("fn main() {}\n");
    std::fs::create_dir_all(root.join("wost/src/boot")).expect("boot fixture directory");
    std::fs::create_dir_all(root.join("kernel/src")).expect("kernel fixture directory");
    std::fs::write(root.join("wost/src/lib.rs"), "pub mod boot;\n").expect("wost fixture root");
    std::fs::write(
        root.join("wost/src/boot.rs"),
        format!(
            "{}\npub fn foundation() {{}}\n",
            validated_entry_fixture_source()
        ),
    )
    .expect("wost boot fixture");
    let kernel = r#"::wost::entry!(kernel_main);
fn kernel_main(_: wost::boot::BootInfo) -> ! {
    wost::boot::foundation();
    loop {}
}
"#;
    std::fs::write(root.join("kernel/src/main.rs"), kernel).expect("kernel entry fixture");
    let target = |package: &str, name: &str, relative: &str, kind, executable| CargoTarget {
        package: package.to_string(),
        package_id: format!("{package}-id"),
        name: name.to_string(),
        relative: PathBuf::from(relative),
        kind,
        test: true,
        executable,
    };
    let targets = vec![
        target(
            "fixture",
            "fixture",
            "libs/fixture/src/main.rs",
            CargoTargetKind::Bin,
            true,
        ),
        target(
            "wost",
            "wost",
            "wost/src/lib.rs",
            CargoTargetKind::Library,
            false,
        ),
        target(
            crate::paths::KERNEL_PACKAGE,
            crate::paths::KERNEL_PACKAGE,
            "kernel/src/main.rs",
            CargoTargetKind::Bin,
            true,
        ),
    ];
    let mut packages = BTreeMap::<String, Vec<CargoTarget>>::new();
    for target in &targets {
        packages
            .entry(target.package.clone())
            .or_default()
            .push(target.clone());
    }
    let catalog = CargoCatalog {
        packages,
        resolved: BTreeMap::from([(
            format!("{}-id", crate::paths::KERNEL_PACKAGE),
            vec![ResolvedDependency {
                name: "wost".to_string(),
                package_id: "wost-id".to_string(),
                normal: true,
                build: false,
                dev: false,
            }],
        )]),
    };
    let raw = Inventory::discover_with_targets(&root, &targets)
        .expect("entry inventory without catalog authority");
    assert!(raw.entry.is_none());
    let inventory =
        Inventory::discover_from_catalog(&root, &catalog).expect("entry-linked inventory");
    let entry = inventory.entry.expect("trusted entry link");
    let foundation = inventory
        .nodes
        .iter()
        .find(|node| node.symbol == "boot::foundation")
        .expect("foundation API");
    let mut decoy_nodes = inventory.nodes.clone();
    let mut decoy = decoy_nodes[entry.invocation_node].clone();
    decoy.id = decoy_nodes.len();
    decoy.range = foundation.range;
    decoy_nodes.push(decoy);
    let mut declaration_decoy = decoy_nodes[entry.macro_declaration].clone();
    declaration_decoy.id = decoy_nodes.len();
    declaration_decoy.macro_declaration = false;
    decoy_nodes.push(declaration_decoy);
    let spec = EntrySpec {
        file: entry.file,
        invocation: entry.invocation,
        macro_path: entry.macro_path,
        argument: entry.argument,
        handler: "kernel_main".to_string(),
    };
    let authority = EntryContractAuthority {
        uses: validate_wost_entry_contract(&root).expect("entry fixture authority"),
    };
    let resolved = resolve_entry_link(
        &decoy_nodes,
        &inventory.files,
        &[spec],
        true,
        Some(&authority),
    )
    .expect("same-symbol decoys outside their exact entry roles are ignored")
    .expect("resolved entry link");
    assert_eq!(resolved.invocation_node, entry.invocation_node);
    assert_eq!(
        inventory.exact_entry_invocation_caller(
            entry.file,
            entry.argument.start,
            entry.function_node,
        ),
        Some(entry.invocation_node)
    );
    assert_eq!(
        inventory.exact_entry_invocation_caller(
            entry.file,
            entry.macro_path.start,
            entry.macro_declaration,
        ),
        Some(entry.invocation_node)
    );
    assert_eq!(
        inventory.exact_entry_invocation_caller(
            entry.file + 1,
            entry.argument.start,
            entry.function_node,
        ),
        None
    );
    assert_eq!(
        inventory.exact_entry_invocation_caller(
            entry.file,
            entry.macro_path.start,
            entry.function_node,
        ),
        None
    );
    assert_eq!(
        inventory.exact_entry_invocation_caller(
            entry.file,
            entry.argument.start,
            entry.macro_declaration,
        ),
        None
    );
    assert_eq!(
        inventory.exact_entry_invocation_caller(entry.file, entry.macro_path.start, foundation.id,),
        None
    );
    assert!(inventory.is_direct_macro_call(
        entry.file,
        entry.macro_path.start,
        entry.macro_declaration,
    ));
    assert!(!inventory.is_direct_macro_call(
        entry.file,
        entry.argument.start,
        entry.macro_declaration,
    ));
    assert!(!inventory.is_direct_macro_call(
        entry.file,
        entry.macro_path.start,
        entry.function_node,
    ));
    let mut ids = BTreeSet::new();
    ids.extend([entry.function_node, entry.macro_declaration, foundation.id]);
    let diagnostic = |at: LineColumn, id| {
        format!(
            "kernel/src/main.rs:{}:{}: warning: resolved use {MARKER}{id}\n",
            at.line,
            at.column + 1
        )
    };
    let mut output = diagnostic(entry.argument.start, entry.function_node);
    output.push_str(&diagnostic(entry.macro_path.start, entry.macro_declaration));
    output.push_str(&diagnostic(
        LineColumn { line: 3, column: 4 },
        foundation.id,
    ));
    let mut uses = Uses::default();
    uses.observe(
        RootKind::Production,
        &inventory,
        &root,
        &output,
        &Instrumentation {
            insertions: BTreeMap::new(),
            ids,
        },
    )
    .expect("trusted entry diagnostics");
    let live = uses.live(&inventory);
    assert!(live.contains(&entry.invocation_node));
    assert!(live.contains(&entry.function_node));
    assert!(live.contains(&entry.macro_declaration));
    assert!(live.contains(&foundation.id));

    let alternate = kernel
        .replace("entry!(kernel_main)", "entry!(other)")
        .replace("fn kernel_main", "fn other");
    std::fs::write(root.join("kernel/src/main.rs"), alternate)
        .expect("malformed kernel entry fixture");
    let error = Inventory::discover_from_catalog(&root, &catalog)
        .expect_err("a different macro argument must not become an entry root");
    assert!(
        format!("{error:#}").contains("public expansion")
            || format!("{error:#}").contains("trusted"),
        "{error:#}"
    );

    let local_entry = r#"#![no_std]
pub mod boot { pub struct BootInfo; }
#[macro_export]
macro_rules! entry {
    ($path:path) => { const _: fn($crate::boot::BootInfo) -> ! = $path; };
}
extern crate self as wost;
::wost::entry!(kernel_main);
fn kernel_main(_: boot::BootInfo) -> ! { loop {} }
"#;
    let generated_binding = r#"#![no_std]
pub mod boot { pub struct BootInfo; }
#[macro_export]
macro_rules! entry {
    ($path:path) => { const _: fn($crate::boot::BootInfo) -> ! = $path; };
}
macro_rules! bind_wost { () => { extern crate self as wost; } }
bind_wost!();
::wost::entry!(kernel_main);
fn kernel_main(_: boot::BootInfo) -> ! { loop {} }
"#;
    for (label, source, refused) in [
        (
            "local wost extern-root binding",
            local_entry,
            "rebinds the kernel entry dependency root",
        ),
        (
            "opaque root item generator",
            generated_binding,
            "opaque root item macro",
        ),
    ] {
        std::fs::write(root.join("kernel/src/main.rs"), source)
            .expect("hostile kernel entry fixture");
        let result = Command::new("rustc")
            .current_dir(&root)
            .args([
                "--edition=2024",
                "--crate-name=kernel_entry_fixture",
                "--crate-type=lib",
                "kernel/src/main.rs",
                "--emit=metadata",
                "--error-format=short",
                "-o",
                "kernel-entry-fixture.rmeta",
            ])
            .output()
            .expect("compile hostile kernel entry fixture");
        assert!(
            result.status.success(),
            "{label} did not compile:\n{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let error = Inventory::discover_from_catalog(&root, &catalog).expect_err(label);
        assert!(format!("{error:#}").contains(refused), "{label}: {error:#}");
    }
    std::fs::remove_dir_all(root).expect("remove entry-link fixture");
}
