use super::*;

use crate::sha256;

type AuditRecord = (&'static str, usize, usize, &'static str);

const AUDITED: &[AuditRecord] = &[
    (
        "wost/src/arch/x86_64/apic.rs",
        3,
        0,
        "8e61d9472c4b24740071ec4845dae30a1c9e5f5801f22b652603c7bf80d5a229",
    ),
    (
        "wost/src/arch/x86_64/cpu/selftest.rs",
        3,
        0,
        "b508aed503c23ba4e0f0eaf43517b3723cf9eef29f4715083daaa8133c942133",
    ),
    (
        "wost/src/arch/x86_64/cpu.rs",
        19,
        3,
        "5c167d8a2c6052ee5afde52b27ff45f788d091d50e64e903345cad9c3e74ebe3",
    ),
    (
        "wost/src/arch/x86_64/interrupt/selftest.rs",
        5,
        0,
        "61cd4851244afe7a2db0078bbcc64d96b8ef78bf77e91c65c3a765976dcd96ac",
    ),
    (
        "wost/src/arch/x86_64/interrupt.rs",
        18,
        2,
        "4518b1f3c0f0368e0ffe264c1d831220273bb108ef2c4233100f11719b8031a6",
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
        "1d986d9001632c215b66a024244cf1a73ea6c4155e136dc9b33bcc2d648852fa",
    ),
    (
        "wost/src/arch/x86_64/paging/map.rs",
        1,
        0,
        "371a0630783fc20c5236e32dd081630fec998cded56b1bd0f6abc072de17e49d",
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
        "5045ebf584fe6e589b8fd26d1c72d479e8b094d9b62df97a139684a2826197f2",
    ),
    (
        "wost/src/arch/x86_64/paging/walk.rs",
        2,
        0,
        "81475ef9cb6de0f0f57abf1d831173fa9cfc42caa6af79dc437aece8c2c52452",
    ),
    (
        "wost/src/arch/x86_64/paging.rs",
        1,
        0,
        "75530d39c34be31923b71111b895ee2214ca8c80f3ca8ce22dbec1a8741db178",
    ),
    (
        "wost/src/arch/x86_64/percpu.rs",
        2,
        0,
        "b6bcf82a97b5f027384c97b04ed72ba82eafb439eb2d3a357a2c76aa614bcae1",
    ),
    (
        "wost/src/arch/x86_64/port.rs",
        16,
        10,
        "0d260694cc20490e307b7939758cc60bd27e0551e1c4147eeaebc3cc769a6518",
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
        "91cc1db000b01a31cbeb4665a04b1e102a1e582f70f9698f685fe34543459ade",
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
        "41288f0856c0cb1724cc0bcfebccc1d71158d79f425b13727efbba50224c8599",
    ),
    (
        "wost/src/arch/x86_64/usercopy.rs",
        5,
        1,
        "c2f09adf3aa42f0d74ae8c89707d16e448ff9f404c53420ff2ba40db16e943e8",
    ),
    (
        "wost/src/arch/x86_64/xstate/selftest.rs",
        3,
        0,
        "0dad190e439ab632ed5b49a12439586c2db3bfe449b29ac2ac48647eea9b7c71",
    ),
    (
        "wost/src/arch/x86_64/xstate.rs",
        6,
        3,
        "68f4111753047a1178581f2a099ad80c7fec2c610802fd5ee7597c6543a066fa",
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
        "946d21b83dff8188700077809b9bccfe2cb5aaeb4c8480044f16bad32c29c42e",
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
        "bca89853d182908f6206352b845eac6687c0434ce78352b348891f229bc17e32",
    ),
    (
        "wost/src/mm/heap.rs",
        6,
        5,
        "34b6c2fad1fdb6ac1f8874f78a038597228a9278bdbf0ec27112fc2f7457341f",
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
        "64516b2617664a8613bc673c50faa60acf9b051e3f9b70d242d164f4e2ad7850",
    ),
    (
        "wost/src/mm.rs",
        1,
        0,
        "ef4c9e74a2edf8471c4655e2b30acf7a73a6db07d89ddc40ed350d6f2f7950eb",
    ),
    (
        "wost/src/mmio.rs",
        2,
        2,
        "d8b01b81f794cf8a315e826e536a1ce24ed5670e19be2aa56f01cb5204ced83f",
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
