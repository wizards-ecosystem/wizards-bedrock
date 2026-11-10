use super::*;

use crate::sha256;

type AuditRecord = (&'static str, usize, usize, &'static str);

const AUDITED: &[AuditRecord] = &[
    (
        "wost/src/arch/x86_64/apic.rs",
        3,
        0,
        "8ed05d10cc30b493cb69e760183efbf8e87e8fbbb8698a8d9f9a876c6111fe9b",
    ),
    (
        "wost/src/arch/x86_64/cpu.rs",
        22,
        3,
        "e543e20b5f5a71452b09b1279ab85f426df40c0a74db1a4a3b5b2c768c4365c6",
    ),
    (
        "wost/src/arch/x86_64/interrupt.rs",
        23,
        2,
        "b65481bcf111bb4b67107c1dd01fb03dd0a7a69fd5fcf690cfee4ce5e0c6d411",
    ),
    (
        "wost/src/arch/x86_64/kpti.rs",
        2,
        0,
        "1f86a1c6569b16f1c934f69134f9eab72109102971c374799d5ab6d6cb5be15e",
    ),
    (
        "wost/src/arch/x86_64/msr.rs",
        2,
        1,
        "2cb6c33ff4f764985b6e0f0fd7a5c1a8bfa1c923ab30f80cf55f233178632b31",
    ),
    (
        "wost/src/arch/x86_64/paging/activation.rs",
        1,
        0,
        "2e1882422e14e766d42720af0ae94550ec13786a6a764af9f1fe9c22a2489595",
    ),
    (
        "wost/src/arch/x86_64/paging/device.rs",
        1,
        0,
        "004685bc88946a447fef8bfe076c347351f93a135814e2f8506f12f88bd3a179",
    ),
    (
        "wost/src/arch/x86_64/paging/map.rs",
        1,
        0,
        "89817fe16ea48ec5667d06eff0599932ea0f70baca3d38249a7c3ebfd8c7b9cd",
    ),
    (
        "wost/src/arch/x86_64/paging/reclaim.rs",
        3,
        0,
        "65ebdda4faa5d199529f1ae84d2eff6dd05f6c8e8c4212f63f122c0710667932",
    ),
    (
        "wost/src/arch/x86_64/paging/selftest.rs",
        5,
        0,
        "80f4cb3092c3513aff2900355e001450713d963fdd1f3bd06a35f07a4e279f1c",
    ),
    (
        "wost/src/arch/x86_64/paging/walk.rs",
        2,
        0,
        "53b41becda40abbf874b98513b4ef764a047627d9c829cb99801f597c5bbd6dd",
    ),
    (
        "wost/src/arch/x86_64/paging.rs",
        1,
        0,
        "2344c421d5516c9eb69355daf70ffb5a16be312d4fddeb9789e1017047bb23ff",
    ),
    (
        "wost/src/arch/x86_64/percpu.rs",
        2,
        0,
        "4cf6b632b37b1be9af9c66a885a9b7e40650a494a69a075ac5e00f4b265d916c",
    ),
    (
        "wost/src/arch/x86_64/port.rs",
        16,
        10,
        "0d260694cc20490e307b7939758cc60bd27e0551e1c4147eeaebc3cc769a6518",
    ),
    (
        "wost/src/arch/x86_64/segment.rs",
        14,
        0,
        "71e5cfef2416ec47e01dd6504544710c7e7138943d6b95ce1a5ebd492c310196",
    ),
    (
        "wost/src/arch/x86_64/shared_memory.rs",
        2,
        2,
        "6143ca421ca29d7c048b84661bdd13768bd3a0d5c40d4746c2831e500304a8d0",
    ),
    (
        "wost/src/arch/x86_64/task.rs",
        14,
        2,
        "e5e26213bedfd484ea89f1de2f4b3aca57099512f24646b508251c926b54ba7a",
    ),
    (
        "wost/src/arch/x86_64/usercopy.rs",
        5,
        1,
        "b1567748dea7bbee1d476f157165ec95632c2b7e7f519b95e7e38d32f5f45d4b",
    ),
    (
        "wost/src/arch/x86_64/xstate.rs",
        9,
        3,
        "1ac37cdf30d686968286279f597fc64cb98a94194320de68f94dfa8d8bb3c527",
    ),
    (
        "wost/src/boot/limine_glue.rs",
        3,
        0,
        "c0eabeea6f0a223d1b095c82b3511658fa82090757e09f55c22ee1bdc5a211b4",
    ),
    (
        "wost/src/boot.rs",
        0,
        1,
        "25e8e2e9dea71d331ba754fbec134d06b9f8e272c123557175d8404d206a21ed",
    ),
    (
        "wost/src/cell.rs",
        0,
        1,
        "92e1579f8d90b74e8d0897dc5a1da8e23b238131d156d7a565b36ae82b6b4ce9",
    ),
    (
        "wost/src/mm/direct.rs",
        6,
        1,
        "d0946cb000e2bb610eb212500e7cbd8c7e531879f4cf817592b2fd1e47f15b2b",
    ),
    (
        "wost/src/mm/frame.rs",
        3,
        2,
        "e3b995d403e9f70382972fb525414b4f59ade0276ec459ee00dfa04a2bfaa037",
    ),
    (
        "wost/src/mm/heap.rs",
        6,
        5,
        "b2d97609f9612aced46b2c9d0becb825b12cb202fbe0357bdefc5c708a65a741",
    ),
    (
        "wost/src/mm/selftest.rs",
        5,
        0,
        "1a7ea52a96d4175f87bd1fa1142e1997fa10964def16b1c2b980e9a08dedce58",
    ),
    (
        "wost/src/mm/shared.rs",
        2,
        0,
        "8bc5feb39e5e98e68fb57adcb1e615bb4d685b4052c5e06835c7b5b37fb2a625",
    ),
    (
        "wost/src/mm.rs",
        1,
        0,
        "ae74ff94eb059f9f246c1937ebd552f7ee6f4f2aec5ebb17e5e13f5f19c9b7df",
    ),
    (
        "wost/src/mmio.rs",
        2,
        2,
        "982617ae9b24dd72bfeeca53ce86e76546b4ee3edc827bd43db5cdc51018a99b",
    ),
    (
        "wost/src/sync.rs",
        4,
        4,
        "184ff84494b31684d1e927bda0b9707999c3df5717d3cd1fe2e0dd33146bdb2d",
    ),
];

fn unsafe_file_inventory(root: &Path) -> Vec<(String, UnsafeFacts, String)> {
    let source_root = root.join("wost/src");
    let mut inventory = Vec::new();
    for source in sources(&source_root).expect("wost source walk") {
        let text = std::fs::read_to_string(&source)
            .unwrap_or_else(|_| panic!("reading {}", source.display()));
        let syntax =
            syn::parse_file(&text).unwrap_or_else(|_| panic!("parsing {}", source.display()));
        let mut facts = UnsafeFacts::default();
        facts.visit_file(&syntax);
        if facts.blocks == 0 && facts.declarations == 0 {
            continue;
        }
        let relative = source
            .strip_prefix(root)
            .expect("source below workspace")
            .to_string_lossy()
            .replace('\\', "/");
        let normalised = text.replace("\r\n", "\n");
        let digest = sha256::hex(&sha256::digest(normalised.as_bytes()));
        inventory.push((relative, facts, digest));
    }
    inventory
}

fn render_inventory(inventory: &[(String, UnsafeFacts, String)]) -> String {
    inventory
        .iter()
        .map(|(path, facts, digest)| {
            format!(
                "    (\"{path}\", {}, {}, \"{digest}\"),",
                facts.blocks, facts.declarations
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_block_is_counted_and_a_declaration_is_not() {
    let body = |source: &str| count(&format!("fn f() {{ {source} }}")).expect("syntax");

    assert_eq!(body("unsafe { one() }").0, 1);
    assert_eq!(count("unsafe fn f() { one() }").expect("syntax"), (0, 0));
    assert_eq!(count("unsafe impl Send for T {}").expect("syntax"), (0, 0));
    assert_eq!(count("// unsafe { not_code() }").expect("syntax"), (0, 0));
    assert_eq!(body("unsafe {\n    if x {\n        y\n    }\n}").0, 5);
    assert_eq!(body("unsafe { a }\nunsafe { b }"), (2, 2));
}

#[test]
fn every_unsafe_source_has_one_reviewed_fingerprint() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");
    let inventory = unsafe_file_inventory(root);
    let rendered = render_inventory(&inventory);

    assert_eq!(
        inventory.len(),
        AUDITED.len(),
        "unsafe source inventory changed; review it and replace AUDITED with:\n{rendered}"
    );
    for ((path, facts, digest), expected) in inventory.iter().zip(AUDITED) {
        assert_eq!(
            (
                path.as_str(),
                facts.blocks,
                facts.declarations,
                digest.as_str()
            ),
            *expected,
            "unsafe source changed; review it and replace AUDITED with:\n{rendered}"
        );
    }
}

#[test]
fn the_unsafe_in_wost_is_inside_its_budget() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");
    let (lines, blocks) = measure(&root.join("wost").join("src")).expect("wost sources");

    assert!(
        lines <= BUDGET,
        "{lines} lines of unsafe in {blocks} blocks, over the budget of {BUDGET}"
    );
    // A near-zero result means the scanner stopped reaching the TCB.
    assert!(lines > 100, "only {lines} lines found; the scan is broken");

    assert_eq!(
        lines, MEASURED,
        "wost holds {lines} lines of unsafe, not the {MEASURED} recorded beside the budget;              update MEASURED and say in the commit what changed"
    );
}

/// `kernel/` has none at all, which `#![deny(unsafe_code)]` enforces — so
/// this is the check that the *counter* agrees with the lint, and would
/// notice an `#[allow]` slipped in beside it.
#[test]
fn the_kernel_has_no_unsafe_at_all() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");
    let (lines, blocks) = measure(&root.join("kernel").join("src")).expect("kernel sources");
    assert_eq!((lines, blocks), (0, 0), "kernel/ grew an unsafe block");
}

/// Check the declarations because counting blocks cannot see an unused
/// `unsafe fn` or `unsafe impl` (D-147).
#[test]
fn the_crate_boundary_is_still_declared() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");

    let mut crates = 0;
    for entry in crate::paths::directory_entries(&root.join("libs"))
        .expect("complete libs directory listing")
    {
        let path = entry.path();
        if !entry.is_dir() {
            continue;
        }
        let lib = path.join("src").join("lib.rs");
        let text =
            std::fs::read_to_string(&lib).unwrap_or_else(|_| panic!("reading {}", lib.display()));
        assert!(
            text.contains("#![forbid(unsafe_code)]"),
            "{} does not forbid unsafe",
            lib.display()
        );
        crates += 1;
    }
    assert!(
        crates > 10,
        "only {crates} libs crates found; the walk is broken"
    );

    // `kernel/` denies rather than forbids, because `wost`'s `entry!`
    // macro expands `#[allow(unsafe_code)]` items into it (D-67).
    let main = std::fs::read_to_string(root.join("kernel/src/main.rs")).expect("main.rs");
    assert!(
        main.contains("#![deny(unsafe_code)]"),
        "kernel/ does not deny unsafe"
    );

    let wost = std::fs::read_to_string(root.join("wost/src/lib.rs")).expect("wost lib.rs");
    assert!(
        wost.contains("#![allow(\n    unsafe_code,\n"),
        "wost no longer lifts the workspace deny; the boundary has moved"
    );
}

#[test]
fn no_libs_crate_has_any_unsafe() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");
    let (lines, blocks) = measure(&root.join("libs")).expect("libs sources");
    assert_eq!(
        (lines, blocks),
        (0, 0),
        "a libs/ crate grew an unsafe block"
    );
}
