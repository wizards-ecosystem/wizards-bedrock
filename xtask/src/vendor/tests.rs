use std::fs::File;

use super::*;
use crate::test_support::create_file_symlink;

const DIGEST: &str = "91ef81c1f95c4ac8bf77c4393a0b2fcd2628550827d7abf89a43194400e17a85";

#[test]
fn parses_the_text_and_binary_forms() {
    let entries = parse(&format!(
        "# a comment\n\n{DIGEST}  plain.efi\n{DIGEST} *binary.efi\n"
    ))
    .unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, "plain.efi");
    assert_eq!(entries[1].name, "binary.efi");
    assert!(entries.iter().all(|entry| entry.digest == DIGEST));
}

#[test]
fn uppercase_digests_are_normalised() {
    let entries = parse(&format!("{}  x.efi", DIGEST.to_ascii_uppercase())).unwrap();
    assert_eq!(entries[0].digest, DIGEST);
}

#[test]
fn rejects_a_truncated_digest() {
    assert!(parse("deadbeef  x.efi").is_err());
}

#[test]
fn rejects_a_line_with_no_file_name() {
    assert!(parse(DIGEST).is_err());
    assert!(parse(&format!("{DIGEST}   ")).is_err());
}

#[test]
fn checksum_names_are_unique_direct_children() {
    assert!(parse(&format!("{DIGEST}  ../outside")).is_err());
    assert!(parse(&format!("{DIGEST}  nested/payload")).is_err());
    assert!(parse(&format!("{DIGEST}  nested\\payload")).is_err());
    assert!(parse(&format!("{DIGEST}  payload\n{DIGEST}  payload\n")).is_err());
}

#[test]
fn checksum_entry_count_is_bounded_independently_of_text_size() {
    let listing: String = (0..=MAX_DIRECTORY_ENTRIES)
        .map(|index| format!("{DIGEST}  payload-{index}\n"))
        .collect();
    let error = parse(&listing).unwrap_err().to_string();
    assert!(error.contains("more than 256 entries"), "{error}");
}

#[test]
fn consumed_payload_authentication_requires_one_exact_adjacent_entry() {
    let root = crate::test_support::reserve_temp_directory("wizard-vendor-consumption");
    let vendor = root.join("vendor");
    std::fs::create_dir(&vendor).unwrap();
    let payload = vendor.join("BOOTX64.EFI");
    let listing = vendor.join("CHECKSUMS");
    let bytes = b"reviewed bootloader";
    std::fs::write(&payload, bytes).unwrap();
    let digest = sha256::hex(&sha256::digest(bytes));

    std::fs::write(&listing, format!("{digest} *BOOTX64.EFI\n")).unwrap();
    verify_payload_bytes(&listing, &payload, bytes).unwrap();

    std::fs::write(
        &listing,
        format!("{digest} *BOOTX64.EFI\n{digest} *BOOTIA32.EFI\n"),
    )
    .unwrap();
    let error = verify_payload_bytes(&listing, &payload, bytes)
        .unwrap_err()
        .to_string();
    assert!(error.contains("exactly one payload digest"), "{error}");

    std::fs::write(&listing, format!("{digest} *BOOTIA32.EFI\n")).unwrap();
    let error = verify_payload_bytes(&listing, &payload, bytes)
        .unwrap_err()
        .to_string();
    assert!(error.contains("instead of the consumed payload"), "{error}");

    let outside = root.join("outside");
    let error = verify_payload_bytes(&listing, &outside, bytes)
        .unwrap_err()
        .to_string();
    assert!(error.contains("is not beside"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn consumed_payload_authentication_refuses_a_redirected_listing() {
    let root = crate::test_support::reserve_temp_directory("wizard-vendor-consumption-link");
    let vendor = root.join("vendor");
    let outside = root.join("outside");
    std::fs::create_dir_all(&vendor).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let bytes = b"substituted bootloader";
    let payload = vendor.join("BOOTX64.EFI");
    std::fs::write(&payload, bytes).unwrap();
    let external_listing = outside.join("CHECKSUMS");
    let digest = sha256::hex(&sha256::digest(bytes));
    let external_bytes = format!("{digest} *BOOTX64.EFI\n");
    std::fs::write(&external_listing, &external_bytes).unwrap();
    let listing = vendor.join("CHECKSUMS");
    create_file_symlink(&external_listing, &listing);

    let result = verify_payload_bytes(&listing, &payload, bytes);
    let retained = std::fs::read(&external_listing).unwrap();
    std::fs::remove_dir_all(&root).unwrap();

    let error = result.expect_err("a redirected checksum listing must be refused");
    assert!(
        format!("{error:#}").contains("not a real file"),
        "unexpected refusal: {error:#}"
    );
    assert_eq!(retained, external_bytes.as_bytes());
}

#[test]
fn oversized_checksum_listings_are_refused_before_allocation() {
    let root = crate::test_support::reserve_temp_directory("wizard-vendor-listing-bound");
    let listing = root.join("CHECKSUMS");
    File::create(&listing)
        .unwrap()
        .set_len(MAX_LISTING_BYTES + 1)
        .unwrap();

    let error = verify_listing(&listing).unwrap_err().to_string();
    assert!(
        error.contains("exceeding the 1048576-byte checksum-listing limit"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn checksum_listing_growth_is_refused_after_measurement() {
    let root = crate::test_support::reserve_temp_directory("wizard-vendor-listing-growth");
    let listing = root.join("CHECKSUMS");
    std::fs::write(&listing, b"x").unwrap();
    let growing = listing.clone();

    let error = read_listing_with(&listing, move || {
        let mut options = std::fs::OpenOptions::new();
        std::fs::OpenOptions::write(&mut options, true);
        std::fs::OpenOptions::open(&options, growing)?.set_len(MAX_LISTING_BYTES + 1)?;
        Ok(())
    })
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("grew beyond the 1048576-byte checksum-listing limit"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn oversized_vendored_payloads_are_refused_before_hashing() {
    let root = crate::test_support::reserve_temp_directory("wizard-vendor-payload-bound");
    let listing = root.join("CHECKSUMS");
    std::fs::write(&listing, format!("{DIGEST}  payload\n")).unwrap();
    File::create(root.join("payload"))
        .unwrap()
        .set_len(MAX_PAYLOAD_BYTES + 1)
        .unwrap();

    let error = verify_listing(&listing).unwrap_err().to_string();
    assert!(
        error.contains("exceeding the 67108864-byte vendored-payload limit"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn vendored_directories_have_an_independent_total_payload_bound() {
    let root = crate::test_support::reserve_temp_directory("wizard-vendor-total-bound");
    let mut listing = String::new();
    for index in 0..5 {
        let name = format!("payload-{index}");
        listing.push_str(&format!("{DIGEST}  {name}\n"));
        File::create(root.join(name))
            .unwrap()
            .set_len(MAX_PAYLOAD_BYTES)
            .unwrap();
    }
    let checksum_path = root.join("CHECKSUMS");
    std::fs::write(&checksum_path, listing).unwrap();

    let error = verify_listing(&checksum_path).unwrap_err().to_string();
    assert!(
        error.contains("exceeding its 268435456-byte limit"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

fn listed(names: &[&str]) -> Vec<Entry> {
    names
        .iter()
        .map(|name| Entry {
            digest: DIGEST.to_string(),
            name: (*name).to_string(),
        })
        .collect()
}

/// The gap a digest cannot close: an added file has no recorded hash to mismatch.
#[test]
fn a_file_no_entry_lists_is_unaccounted_for() {
    let entries = listed(&["BOOTX64.EFI"]);

    assert_eq!(
        unaccounted(["BOOTX64.EFI", "BOOTIA32.EFI"], &entries),
        ["BOOTIA32.EFI"]
    );
}

#[test]
fn a_directory_holding_exactly_what_is_listed_is_accounted_for() {
    let entries = listed(&["BOOTX64.EFI"]);

    assert!(unaccounted(["BOOTX64.EFI"], &entries).is_empty());
    assert!(
        unaccounted([], &entries).is_empty(),
        "absence is the digest's job"
    );
}

/// `CHECKSUMS` included, since it could not hash itself anyway.
#[test]
fn paperwork_needs_no_digest() {
    let entries = listed(&["BOOTX64.EFI"]);
    let present = ["BOOTX64.EFI", "CHECKSUMS", "LICENSE", "README.md"];

    assert!(unaccounted(present, &entries).is_empty());
}

/// All at once, so fixing them is one pass rather than one commit per file.
#[test]
fn every_stray_is_reported_in_a_stable_order() {
    let entries = listed(&["BOOTX64.EFI"]);

    assert_eq!(
        unaccounted(["zebra.efi", "BOOTX64.EFI", "alpha.efi"], &entries),
        ["alpha.efi", "zebra.efi"]
    );
}

/// `verify` walks a hand-written array; any directory holding a `CHECKSUMS` must be in it.
#[test]
fn every_vendored_directory_is_verified() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");

    let listed = [
        root.join("boot").join("limine").join("CHECKSUMS"),
        root.join("tools").join("tla").join("CHECKSUMS"),
        root.join("tools").join("fat32-corpus").join("CHECKSUMS"),
    ];

    let mut found = Vec::new();
    let mut pending = vec![root.join("boot"), root.join("tools")];
    while let Some(next) = pending.pop() {
        for entry in crate::paths::directory_entries(&next).unwrap_or_else(|reason| {
            panic!(
                "complete directory listing for {}: {reason}",
                next.display()
            )
        }) {
            let path = entry.path();
            if entry.is_dir() {
                pending.push(path);
            } else if path.file_name().is_some_and(|name| name == "CHECKSUMS") {
                found.push(path);
            }
        }
    }

    for checksums in &found {
        assert!(
            listed.iter().any(|known| known == checksums),
            "{} is not one `verify` checks",
            checksums.display()
        );
    }
    assert_eq!(found.len(), listed.len(), "a listing has no directory");
}
