use std::io::Cursor;

use fatfs::FsOptions;

use super::*;

fn test_root(label: &str) -> std::path::PathBuf {
    crate::test_support::reserve_temp_directory(&format!("wizard-esp-{label}"))
}

#[cfg(unix)]
fn create_file_symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("creating ESP source symlink");
}

#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) {
    std::os::windows::fs::symlink_file(target, link).expect("creating ESP source symlink");
}

#[test]
fn redirected_image_sources_are_refused_before_copy() {
    let tree = test_root("redirected-source");
    let outside = tree.join("outside");
    let source_parent = tree.join("checkout");
    std::fs::create_dir(&outside).unwrap();
    std::fs::create_dir(&source_parent).unwrap();
    let sentinel = outside.join("sentinel");
    let redirected = source_parent.join("limine.conf");
    std::fs::write(&sentinel, b"outside authority").unwrap();
    create_file_symlink(&sentinel, &redirected);

    let mut volume = Cursor::new(vec![0u8; 1024 * 1024]);
    format(&mut volume, FatType::Fat16).unwrap();
    let filesystem = FileSystem::new(volume, FsOptions::new()).unwrap();
    let root = filesystem.root_dir();

    let result = copy_into(
        &root,
        "limine.conf",
        &redirected,
        MAX_LIMINE_CONFIG_BYTES,
        "Limine configuration",
    );
    let sentinel_bytes = std::fs::read(&sentinel).unwrap();
    drop(root);
    drop(filesystem);
    std::fs::remove_dir_all(&tree).unwrap();

    let error = result.expect_err("a redirected image source must be refused");
    assert!(
        format!("{error:#}").contains("not a real file"),
        "unexpected refusal: {error:#}"
    );
    assert_eq!(sentinel_bytes, b"outside authority");
}

#[test]
fn oversized_image_sources_are_refused_before_copy() {
    let tree = test_root("oversized-source");
    let source = tree.join("limine.conf");
    let file = std::fs::File::create(&source).unwrap();
    file.set_len(MAX_LIMINE_CONFIG_BYTES + 1).unwrap();

    let mut volume = Cursor::new(vec![0u8; 1024 * 1024]);
    format(&mut volume, FatType::Fat16).unwrap();
    let filesystem = FileSystem::new(volume, FsOptions::new()).unwrap();
    let root = filesystem.root_dir();

    let result = copy_into(
        &root,
        "limine.conf",
        &source,
        MAX_LIMINE_CONFIG_BYTES,
        "Limine configuration",
    );
    let destination_is_absent = root.open_file("limine.conf").is_err();
    drop(root);
    drop(filesystem);
    std::fs::remove_dir_all(&tree).unwrap();

    let error = result.expect_err("an oversized image source must be refused");
    assert!(
        format!("{error:#}").contains("65536-byte allocation limit"),
        "unexpected refusal: {error:#}"
    );
    assert!(destination_is_absent);
}

#[test]
fn direct_bounded_image_sources_are_copied_exactly() {
    use std::io::Read;

    let tree = test_root("direct-source");
    let source = tree.join("limine.conf");
    let expected = b"timeout: 0\n/Wizard\n    protocol: limine\n";
    std::fs::write(&source, expected).unwrap();

    let mut volume = Cursor::new(vec![0u8; 1024 * 1024]);
    format(&mut volume, FatType::Fat16).unwrap();
    let filesystem = FileSystem::new(volume, FsOptions::new()).unwrap();
    let root = filesystem.root_dir();
    copy_into(
        &root,
        "limine.conf",
        &source,
        MAX_LIMINE_CONFIG_BYTES,
        "Limine configuration",
    )
    .unwrap();

    let mut copied = Vec::new();
    root.open_file("limine.conf")
        .unwrap()
        .read_to_end(&mut copied)
        .unwrap();
    drop(root);
    drop(filesystem);
    std::fs::remove_dir_all(&tree).unwrap();
    assert_eq!(copied, expected);
}

fn populate_bootloader_fixture(
    label: &str,
    payload: &[u8],
    recorded_payload: &[u8],
) -> (Result<()>, Option<Vec<u8>>) {
    use std::io::Read;

    let tree = test_root(label);
    let checkout = tree.join("checkout");
    let limine = checkout.join("boot").join("limine");
    std::fs::create_dir_all(&limine).unwrap();
    let recorded = crate::sha256::hex(&crate::sha256::digest(recorded_payload));
    std::fs::write(limine.join("BOOTX64.EFI"), payload).unwrap();
    std::fs::write(
        limine.join("CHECKSUMS"),
        format!("{recorded} *BOOTX64.EFI\n"),
    )
    .unwrap();
    std::fs::write(checkout.join("boot").join("limine.conf"), b"timeout: 0\n").unwrap();

    let paths = Paths::new(checkout, None);
    let mut volume = Cursor::new(vec![0u8; 1024 * 1024]);
    format(&mut volume, FatType::Fat16).unwrap();
    let filesystem = FileSystem::new(volume, FsOptions::new()).unwrap();
    let result = populate(&filesystem, &paths, b"kernel");
    let copied = filesystem
        .root_dir()
        .open_dir("EFI")
        .and_then(|efi| efi.open_dir("BOOT"))
        .and_then(|boot| boot.open_file("BOOTX64.EFI"))
        .ok()
        .map(|mut file| {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).unwrap();
            bytes
        });
    drop(filesystem);
    std::fs::remove_dir_all(&tree).unwrap();
    (result, copied)
}

#[test]
fn image_population_refuses_a_bootloader_that_misses_its_pinned_digest() {
    let (result, copied) = populate_bootloader_fixture(
        "substituted-bootloader",
        b"substituted bootloader",
        b"reviewed bootloader",
    );

    let error = result.expect_err("a substituted bootloader must be refused");
    assert!(
        format!("{error:#}").contains("does not match its recorded digest"),
        "unexpected refusal: {error:#}"
    );
    assert!(
        copied.is_none(),
        "unverified bootloader bytes were embedded"
    );
}

#[test]
fn image_population_embeds_the_exact_authenticated_bootloader() {
    let bootloader = b"reviewed bootloader";
    let (result, copied) =
        populate_bootloader_fixture("authenticated-bootloader", bootloader, bootloader);

    result.expect("the pinned bootloader should populate the image");
    assert_eq!(copied, Some(bootloader.to_vec()));
}
