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
        "06a79c948761eebed24e9fa2d6cd493a6edcc92faef96c108c3cffaf3250e557",
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
        "e656955d81c65b526ccf73dbe2f0a01361e0104d34201a8a8c79411a1d8e00c6",
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
        "a56b6c48f91d37316ec91ad98dc9418e560dd2a9bb6830b5107e8ed4b7e65b3c",
    ),
    (
        "wost/src/arch/x86_64/paging/device.rs",
        1,
        0,
        "3bcfe8a135d3aefb7997732e8480590fb0569b3e710fba9f7ee452d412ff4041",
    ),
    (
        "wost/src/arch/x86_64/paging/map.rs",
        1,
        0,
        "dc8a6ae069a199d5624c7c46351ec759426d766224c059fe9edfd34448a0f6f0",
    ),
    (
        "wost/src/arch/x86_64/paging/reclaim.rs",
        3,
        0,
        "671006a87518686d1897a55c0d1bf6167fd01b1c9e84468c24099db32851b2fb",
    ),
    (
        "wost/src/arch/x86_64/paging/selftest.rs",
        5,
        0,
        "ae337408a25b0505c1aeeea9acdf40d751313dcf26c4512cacbc6366b1005382",
    ),
    (
        "wost/src/arch/x86_64/paging/walk.rs",
        2,
        0,
        "fb630daa7909b33ad473f56af874021d26b7f1b1dd4f01bdb42578895dc571d4",
    ),
    (
        "wost/src/arch/x86_64/paging.rs",
        1,
        0,
        "4e78adf5f528e0e796afd58befefc41452ecd7a595275886c9e9a4f08e6b41b6",
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
        "7d91f7aba0dcec7fec0fcc073019488048f538dabad4fc9d68d261303e9c1e8a",
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
        "82675d87bbd8a07768526277b368444560f31164fa4b8049184eb1be5489752e",
    ),
    (
        "wost/src/arch/x86_64/usercopy.rs",
        5,
        1,
        "8ee4552dc01709fa0c3a44036adbf164a0692ef14563731230d99f46ddeecd57",
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
        "e5a12d49379429d88f984b3c7ea5c5e539452aaac19474aba5562f640d742329",
    ),
    (
        "wost/src/mm/frame.rs",
        3,
        2,
        "fab9c367d76e7186bccc43de7db06a17bc83b262dc17852be1385f7dd8cf0a60",
    ),
    (
        "wost/src/mm/heap.rs",
        6,
        5,
        "9b10f868b2a1b2756ecf89cd47f39473cde6ce84010f4444c539d26e23d40b0b",
    ),
    (
        "wost/src/mm/selftest.rs",
        5,
        0,
        "d83b9ffc9a876e645a643607d18bd5ac8ecbd69f83ce2410a3e68caf3970f715",
    ),
    (
        "wost/src/mm/shared.rs",
        2,
        0,
        "e53f0cb3a0fce87c721fc409e194bf798d1082d0f0c6a605a8bff0348c602579",
    ),
    (
        "wost/src/mm.rs",
        1,
        0,
        "2e224c868aae3cd70175cee2126d40f566107c5aafe7c0e3464aae8c66b31f15",
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

// The lint already denies it; this checks that the counter agrees and notices a slipped `#[allow]`.
#[test]
fn the_kernel_has_no_unsafe_at_all() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");
    let (lines, blocks) = measure(&root.join("kernel").join("src")).expect("kernel sources");
    assert_eq!((lines, blocks), (0, 0), "kernel/ grew an unsafe block");
}

// Counting blocks cannot see an unused `unsafe fn` or `unsafe impl`, so the declarations are read.
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

    // `kernel/` denies rather than forbids: `wost`'s `entry!` expands `#[allow(unsafe_code)]` in.
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
