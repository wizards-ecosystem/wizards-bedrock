use super::*;

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root")
}

/// Every crate that states a page size states the same one.
#[test]
fn every_page_size_in_the_tree_is_the_same_page_size() {
    let found = page_size_definitions(root()).expect("the source trees are readable");

    // A regex that stops matching is the failure that reads as a pass:
    // nothing found means nothing compared, and this reports success.
    assert!(
        found.len() >= 5,
        "only {} page-size definitions were found, so the scan is wrong: {found:#?}",
        found.len()
    );

    let (first, rest) = found.split_first().expect("at least one definition");
    for other in rest {
        assert_eq!(
            other.value,
            first.value,
            "{}:{} says {} is {}, {}:{} says {} is {}",
            other.file,
            other.line,
            other.name,
            other.value,
            first.file,
            first.line,
            first.name,
            first.value
        );
    }

    // The hardware's page size, not merely a number five files agree on:
    // they would agree if all five were wrong together.
    assert_eq!(first.value, 4096, "a page is 4096 bytes on x86-64");
}

#[test]
fn a_hexadecimal_page_size_is_read_as_the_same_number() {
    let mut found = Vec::new();
    scan(
        "made/up.rs",
        "pub const PAGE_SIZE: u64 = 0x1_000;\nconst FRAME_SIZE: usize = 4096;\n",
        &mut found,
    )
    .expect("both lines state a literal");
    assert_eq!(found.len(), 2, "{found:#?}");
    assert_eq!(found[0].value, 4096);
    assert_eq!(found[1].value, 4096);
}

#[test]
fn a_page_size_that_is_computed_rather_than_stated_is_refused() {
    let mut found = Vec::new();
    let refused = scan(
        "made/up.rs",
        "pub const PAGE_SIZE: u64 = 1 << 12;\n",
        &mut found,
    );
    assert!(
        refused.is_err(),
        "a computed page size cannot be compared and must not pass quietly"
    );
}
