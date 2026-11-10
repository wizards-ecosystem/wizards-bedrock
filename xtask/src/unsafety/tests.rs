use super::*;

use crate::sha256;

type AuditRecord = (&'static str, usize, usize, &'static str);

const AUDITED: &[AuditRecord] = &[
    (
        "wost/src/arch/x86_64/apic.rs",
        3,
        0,
        "457b091921a975f7646f3907a9938336449d53d87c5fa900d12fc58a3d9fe57c",
    ),
    (
        "wost/src/arch/x86_64/cpu/selftest.rs",
        3,
        0,
        "89f22b42087d4c1534c5033fbbaaae6609238b85be426a93d69aa762aa3c5069",
    ),
    (
        "wost/src/arch/x86_64/cpu.rs",
        19,
        3,
        "27bf4f0b248bff8e656a29be5212af65701c2cababaa232248dda5a7430cf33c",
    ),
    (
        "wost/src/arch/x86_64/interrupt/selftest.rs",
        5,
        0,
        "85985e4fd6349da23fe9006150ce9e84e61a97a14f341a59b8fc4363ef254773",
    ),
    (
        "wost/src/arch/x86_64/interrupt.rs",
        18,
        2,
        "08a9ed8e585ce741be685dd5105d4f850ccb64ad8cd774ca0cec330fa80e173a",
    ),
    (
        "wost/src/arch/x86_64/kpti/selftest.rs",
        2,
        0,
        "5a6756e3b08d545ea84c86065565d8e58a567849e102c22673ae69070b5d8967",
    ),
    (
        "wost/src/arch/x86_64/msr.rs",
        2,
        1,
        "407d0484139a54dbe47474bdbac71960767a254d9e18ef51f28bb305a6014cd4",
    ),
    (
        "wost/src/arch/x86_64/paging/activation.rs",
        1,
        0,
        "8a2120202896981775a1fccf584d7a0b6940af7c1077720155fe6926ae7f236a",
    ),
    (
        "wost/src/arch/x86_64/paging/device.rs",
        1,
        0,
        "8afab4a2af6c2394eb4e66f0c60152f0dc38bcbfb69803857f6f99c629033c2b",
    ),
    (
        "wost/src/arch/x86_64/paging/map.rs",
        1,
        0,
        "ee2e0435648fd6da715d9b9ac5f05fbc2c70fcfa7f46ce9e4c3e69fb51c2ad58",
    ),
    (
        "wost/src/arch/x86_64/paging/reclaim.rs",
        3,
        0,
        "518fdff8ec2e3c52c9ae55b5e0f91baf146eb71f5f5d324f6217155de0cab1c8",
    ),
    (
        "wost/src/arch/x86_64/paging/selftest.rs",
        5,
        0,
        "0364c216d3865e63c16aeb85f72cec812d50b0b94e490017717ab600b5a016fb",
    ),
    (
        "wost/src/arch/x86_64/paging/walk.rs",
        2,
        0,
        "14d75d80eb3ef1eec7b8fc7d698a44d5974214d9fb326f6359510c883e123502",
    ),
    (
        "wost/src/arch/x86_64/paging.rs",
        1,
        0,
        "736f1fa9c8d59fbf631ea19c10a1dc5765d6b50a09fa8b23b1d2de5446b37c42",
    ),
    (
        "wost/src/arch/x86_64/percpu.rs",
        2,
        0,
        "46559faaf62aeca696b53789125d72b8c87210d92c82baa272bd063de62d1429",
    ),
    (
        "wost/src/arch/x86_64/port.rs",
        16,
        10,
        "5e5e10a9998931c7db7200f0540941d2f3d0ff47a4b6a4bfd5ffbedb4ff86fa8",
    ),
    (
        "wost/src/arch/x86_64/segment/selftest.rs",
        3,
        0,
        "5b3a10ced4579fdd002cc7eb455a201e6797ec19ef774e9e7353cff552a9f062",
    ),
    (
        "wost/src/arch/x86_64/segment.rs",
        11,
        0,
        "be3f210bab1821404aea54d700adb26ca7135f73e24da2cdacab2a32098355b8",
    ),
    (
        "wost/src/arch/x86_64/shared_memory.rs",
        2,
        2,
        "cbddb2df8dd96caa3f74d6322fd5a367307db1bc06d2241c2a336103abad6f7c",
    ),
    (
        "wost/src/arch/x86_64/task.rs",
        14,
        2,
        "a0fb58e15e2303aadae86602bda429d6b479d16c19bd621b6bcf5742e7cbe356",
    ),
    (
        "wost/src/arch/x86_64/usercopy.rs",
        5,
        1,
        "0da087f637bbf6c348a3622d8310e989c9d3da27738ba47d3b481a45b0e55d0e",
    ),
    (
        "wost/src/arch/x86_64/xstate/selftest.rs",
        3,
        0,
        "c2c92eecba90b107820cdf1ea5acf4873a23f1363cf37b60ad18917e549c5d81",
    ),
    (
        "wost/src/arch/x86_64/xstate.rs",
        6,
        3,
        "14ee51b5ad731b63a5ed1a2e9c44ae92f67c319b920793142d3d4a6482ddc625",
    ),
    (
        "wost/src/boot/limine_glue.rs",
        3,
        0,
        "338f9c1cb8276628abddb251e57d4199c7614385004f0f13979b4bacb996c546",
    ),
    (
        "wost/src/boot.rs",
        0,
        1,
        "b2b3501e4903b76729ee1976875d16183dfdf56a3634bf9c49bd8155bf58ab8a",
    ),
    (
        "wost/src/cell.rs",
        0,
        1,
        "b2a64df94d580ba80b74d10c029c2dd541dab699419e31647bf15a49835eb4aa",
    ),
    (
        "wost/src/mm/direct.rs",
        6,
        1,
        "6a0bbe85e2771ce23c9fc41f82215564a9b3bfbcc80cd95f5e81bb0bb76dbb39",
    ),
    (
        "wost/src/mm/frame.rs",
        3,
        2,
        "037c1051eb93bcbe642e79d4f71e414318861e660b62d75eb43751d40aba7f0a",
    ),
    (
        "wost/src/mm/heap.rs",
        6,
        5,
        "5ac5c9e834b5891cb402bb433d68a991ffa1ed3fc2dd15d6d34150cfb262375f",
    ),
    (
        "wost/src/mm/selftest.rs",
        5,
        0,
        "791e050d4a47f4f2333065a744e3faad99ec7360918c72aec3f611b0120aab87",
    ),
    (
        "wost/src/mm/shared.rs",
        2,
        0,
        "cec37a1a451efc3e05f5b14e8a498518e01f157f51d5aede0b82a18d02dfb030",
    ),
    (
        "wost/src/mm.rs",
        1,
        0,
        "1ba58092f84a435f9befe794e2e2fde909f50508bde9479924b0aa53f9dbe25f",
    ),
    (
        "wost/src/mmio.rs",
        2,
        2,
        "d63b7d820b38a4526b3c4433d886866f5bf8d0bd83a774ecd4364ecb6582a82e",
    ),
    (
        "wost/src/sync.rs",
        4,
        4,
        "2537badb41ed424f65db0c0d80bb0e8c0d7bac00a49ab2d37720d2aa6ae2ec75",
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
