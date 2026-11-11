use super::{Caught, Mutation};

pub(super) const MUTATIONS: &[Mutation] = &[
    Mutation {
        id: 295,
        file: "xtask/templates/bedrock.vmx.in",
        find: "nvme0.present = \"TRUE\"\nnvme1.present = \"TRUE\"",
        replace: "nvme0.present = \"TRUE\"\nnvme0:0.present = \"TRUE\"\nnvme1.present = \"TRUE\"",
        breaks: "the VMware machine offers a candidate worth retiring",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::the_vmx_boots_efi_and_logs_serial_to_a_file",
        },
    },
    Mutation {
        id: 381,
        file: "xtask/src/cargo.rs",
        find: "        .env(\"CARGO_TARGET_DIR\", child_process_path(paths.target_dir()));",
        replace: "        .env_remove(\"CARGO_TARGET_DIR\");",
        breaks: "isolated cargo builds write outside their private target directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cargo::tests::a_cargo_command_uses_the_paths_private_target",
        },
    },
    Mutation {
        id: 440,
        file: "xtask/src/image.rs",
        find: "            Self::SelfTest => &[paths::SELFTEST_FEATURE],",
        replace: "            Self::SelfTest => &[],",
        breaks: "the self-test image selector compiles the shipped kernel",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::tests::selectors_build_only_their_named_kernel",
        },
    },
    Mutation {
        id: 441,
        file: "xtask/src/image.rs",
        find: "            .open_file(paths::KERNEL_ON_ESP)",
        replace: "            .open_file(paths::PROGRAM_ON_ESP)",
        breaks: "image verification reads another payload instead of the kernel",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::tests::both_kernel_kinds_are_read_back_and_identified",
        },
    },
    Mutation {
        id: 466,
        file: "xtask/src/cargo.rs",
        find: "    let mut args = vec![\"build\".to_string(), \"--locked\".to_string()];",
        replace: "    let mut args = vec![\"build\".to_string()];",
        breaks: "a release build may resolve dependencies outside Cargo.lock",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cargo::tests::kernel_builds_are_locked_to_the_recorded_dependencies",
        },
    },
    Mutation {
        id: 484,
        file: "xtask/src/supply_chain.rs",
        find: "        matches.len() == 1,",
        replace: "        !matches.is_empty(),",
        breaks: "an ambiguous locked dependency silently selects one version",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::a_dependency_with_two_locked_versions_must_name_which_one",
        },
    },
    Mutation {
        id: 490,
        file: "xtask/src/image/program.rs",
        find: "    const COMPLETION_TAIL_AT: u8 = 28;",
        replace: "    const COMPLETION_TAIL_AT: u8 = 24;",
        breaks: "the real ring client acquires its own head instead of the completion tail",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::program::tests::the_ring_client_orders_shared_memory",
        },
    },
    Mutation {
        id: 518,
        file: "xtask/src/image.rs",
        find: "            let sequence = NEXT_RUN_IMAGE.fetch_add(1, Ordering::Relaxed);",
        replace: "            let sequence = NEXT_RUN_IMAGE.load(Ordering::Relaxed);",
        breaks: "concurrent QEMU builds reserve one boot-image path",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::tests::two_qemu_images_are_private_and_removed",
        },
    },
    Mutation {
        id: 536,
        file: "xtask/src/image/program.rs",
        find: "    code.extend_from_slice(&[0x41, 0xB8, 1, 0, 0, 0]);",
        replace: "    code.extend_from_slice(&[0x41, 0xB8, 0, 0, 0, 0]);",
        breaks: "the measured ring behaviour never arms its timestamp path",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::abi::the_c_header_and_the_kernel_agree_about_the_abi",
        },
    },
    Mutation {
        id: 637,
        file: "xtask/src/elf.rs",
        find: "    if stride != SECTION_HEADER_BYTES {",
        replace: "    if false && stride != SECTION_HEADER_BYTES {",
        breaks: "section headers can alias through a zero or non-ELF64 stride",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "elf::tests::a_section_header_stride_is_exactly_elf64s",
        },
    },
    Mutation {
        id: 725,
        file: "xtask/src/sha256.rs",
        find: "            if self.buffered != BLOCK_SIZE {",
        replace: "            if false && self.buffered != BLOCK_SIZE {",
        breaks: "an incremental SHA-256 update discards a partial block before it is complete",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "sha256::tests::incremental_hashing_matches_every_adversarial_block_split",
        },
    },
    Mutation {
        id: 732,
        file: "xtask/src/supply_chain.rs",
        find: "    let publication = write_spdx_stage(&staging, bytes).and_then(|()| {",
        replace: "    std::fs::write(destination, bytes)\n        .with_context(|| format!(\"writing old SPDX document {}\", destination.display()))?;\n    let publication = write_spdx_stage(&staging, bytes).and_then(|()| {",
        breaks: "SPDX publication writes through a preexisting hardlink before replacing its name",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::spdx_publication_never_writes_through_an_external_link",
        },
    },
    Mutation {
        id: 798,
        file: "xtask/src/image.rs",
        find: r#"pub(crate) fn qemu_run_directory(target: &Path) -> Result<PathBuf> {
    crate::mutate::workspace::prepare_real_directory_tree(
        &target.join(paths::QEMU_RUN_DIRECTORY),
        "QEMU private run directory",
    )
}"#,
        replace: r#"pub(crate) fn qemu_run_directory(target: &Path) -> Result<PathBuf> {
    let directory = target.join(paths::QEMU_RUN_DIRECTORY);
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("creating {}", directory.display()))?;
    Ok(directory)
}"#,
        breaks: "private boot images can be created through a redirected QEMU run root",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::tests::private_qemu_images_refuse_a_redirected_run_root_before_writing",
        },
    },
    Mutation {
        id: 804,
        file: "xtask/src/image.rs",
        find: r#"    let canonical_parent = crate::mutate::workspace::prepare_real_directory_tree(
        parent,
        "boot-image target directory",
    )?;"#,
        replace: r#"    let canonical_parent = std::fs::canonicalize(parent)
        .with_context(|| format!("canonicalizing {}", parent.display()))?;"#,
        breaks: "an authoritative boot image can be published through a redirected target directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::tests::authoritative_images_refuse_a_redirected_target_before_writing",
        },
    },
    Mutation {
        id: 805,
        file: "xtask/src/image.rs",
        find: r#"        crate::mutate::workspace::publish_staged_file(
            &canonical_parent,
            &staging,
            &published_path,
            "boot image",
            |published| {
                let digest = crate::sha256::digest_reader(
                    crate::mutate::workspace::open_real_file(published, "boot image")?,
                )
                .with_context(|| format!("hashing boot image {}", destination.display()))?;
                ensure!(
                    digest == staged,
                    "published boot image differs from its complete stage"
                );
                Ok(())
            },
        )?;"#,
        replace: r#"        let _unchecked_digest = staged;
        std::fs::copy(&staging, &published_path)
            .with_context(|| format!("overwriting {}", destination.display()))?;"#,
        breaks: "boot-image publication writes staged bytes through an external hardlink",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::tests::authoritative_image_publication_never_writes_through_an_external_link",
        },
    },
    Mutation {
        id: 811,
        file: "xtask/src/supply_chain.rs",
        find: r#"                digest: package.checksum.clone().with_context(|| {
                    format!(
                        "registry package {} {} has no validated checksum",
                        package.name, package.version
                    )
                })?,"#,
        replace: "                digest: package.checksum.clone().unwrap_or_default(),",
        breaks: "a registry package without a validated checksum enters the inventory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::registry_inventory_projection_refuses_a_missing_checksum",
        },
    },
    Mutation {
        id: 901,
        file: "xtask/src/supply_chain.rs",
        find: "        bytes.len() as u64 <= MAX_SPDX_BYTES,",
        replace: "        bytes.len() as u64 <= MAX_SPDX_BYTES + 1,",
        breaks: "an SPDX rendering one byte beyond its publication ceiling is staged",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::oversized_spdx_renderings_are_refused_before_staging",
        },
    },
    Mutation {
        id: 902,
        file: "xtask/src/supply_chain.rs",
        find: "            after_publish(published)",
        replace: "            let _ = published;\n            Ok(())",
        breaks: "SPDX publication skips its late verifier and cannot roll back its refusal",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::a_late_spdx_publication_refusal_restores_the_previous_document",
        },
    },
    Mutation {
        id: 903,
        file: "xtask/src/supply_chain.rs",
        find: "    ensure!(\n        initial <= maximum,\n        \"{label} is {initial} bytes, exceeding its {maximum}-byte allocation limit\"\n    );",
        replace: "    ensure!(\n        true,\n        \"{label} is {initial} bytes, exceeding its {maximum}-byte allocation limit\"\n    );",
        breaks: "supply-chain text allocates from an already oversized file",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::oversized_supply_chain_text_is_refused_before_allocation",
        },
    },
    Mutation {
        id: 904,
        file: "xtask/src/supply_chain.rs",
        find: "    ensure!(\n        bytes.len() as u64 <= maximum,\n        \"{label} grew beyond its {maximum}-byte allocation limit\"\n    );",
        replace: "    ensure!(\n        true,\n        \"{label} grew beyond its {maximum}-byte allocation limit\"\n    );",
        breaks: "supply-chain text can grow beyond its allocation ceiling during the read",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::supply_chain_text_growth_is_refused_after_measurement",
        },
    },
    Mutation {
        id: 905,
        file: "xtask/src/supply_chain.rs",
        // D-344's opened-file primitive made deleting only the earlier path
        // check equivalent. Remove the complete pre-read refusal (D-398).
        find: "    crate::mutate::workspace::require_direct_file(parent, &path, label)?;\n    let mut file = crate::mutate::workspace::open_real_file(&path, label)?;",
        replace: "    let mut file = std::fs::File::open(&path)\n        .with_context(|| format!(\"opening {}\", path.display()))?;",
        breaks: "supply-chain text follows a redirected file before opening it",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::supply_chain_text_rejects_a_redirect_before_opening",
        },
    },
    Mutation {
        id: 906,
        file: "xtask/src/supply_chain.rs",
        find: "    crate::mutate::workspace::require_direct_file(parent, &path, label)?;\n    String::from_utf8(bytes)",
        replace: "    String::from_utf8(bytes)",
        breaks: "supply-chain text does not revalidate its direct file after reading",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::supply_chain_text_revalidates_its_path_after_reading",
        },
    },
    Mutation {
        id: 907,
        file: "xtask/src/vendor.rs",
        find: "            matches!(components.next(), Some(Component::Normal(_)))\n                && components.next().is_none()\n                && !name.contains(['/', '\\\\'])\n                && !name.chars().any(char::is_control),",
        replace: "            true,",
        breaks: "a vendor checksum entry can name an indirect or control-bearing path",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vendor::tests::checksum_names_are_unique_direct_children",
        },
    },
    Mutation {
        id: 908,
        file: "xtask/src/vendor.rs",
        find: "            names.insert(name.to_string()),",
        replace: "            names.insert(name.to_string()) || names.contains(name),",
        breaks: "a vendor checksum listing accepts the same payload name twice",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vendor::tests::checksum_names_are_unique_direct_children",
        },
    },
    Mutation {
        id: 909,
        file: "xtask/src/vendor.rs",
        find: "            entries.len() < MAX_DIRECTORY_ENTRIES,",
        replace: "            true,",
        breaks: "a vendor checksum listing has no independent entry-count ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vendor::tests::checksum_entry_count_is_bounded_independently_of_text_size",
        },
    },
    Mutation {
        id: 910,
        file: "xtask/src/vendor.rs",
        find: "        initial <= MAX_LISTING_BYTES,",
        replace: "        true,",
        breaks: "a vendor checksum listing allocates from an already oversized file",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vendor::tests::oversized_checksum_listings_are_refused_before_allocation",
        },
    },
    Mutation {
        id: 911,
        file: "xtask/src/vendor.rs",
        find: "        bytes.len() as u64 <= MAX_LISTING_BYTES,",
        replace: "        true,",
        breaks: "a vendor checksum listing can grow beyond its allocation ceiling while reading",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vendor::tests::checksum_listing_growth_is_refused_after_measurement",
        },
    },
    Mutation {
        id: 912,
        file: "xtask/src/vendor.rs",
        find: "            length <= MAX_PAYLOAD_BYTES,",
        replace: "            true,",
        breaks: "an oversized vendored payload reaches the hashing stream",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vendor::tests::oversized_vendored_payloads_are_refused_before_hashing",
        },
    },
    Mutation {
        id: 913,
        file: "xtask/src/vendor.rs",
        find: "            total <= MAX_DIRECTORY_PAYLOAD_BYTES,",
        replace: "            true,",
        breaks: "a vendored directory has no aggregate payload-byte ceiling",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vendor::tests::vendored_directories_have_an_independent_total_payload_bound",
        },
    },
    Mutation {
        id: 1067,
        file: "xtask/src/cargo.rs",
        find: "            Prefix::VerbatimDisk(drive) => PathBuf::from(format!(\"{}:\\\\\", char::from(drive))),",
        replace: "            Prefix::VerbatimDisk(_) => return path.to_path_buf(),",
        breaks: "Cargo receives a Windows verbatim target path it cannot use",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cargo::tests::a_cargo_command_sheds_the_windows_verbatim_target_prefix",
        },
    },
    Mutation {
        id: 1182,
        file: "xtask/src/image.rs",
        find: "    build(staging, &mut disk)?;",
        replace: "    let _ignored_construction_failure = build(staging, &mut disk);",
        breaks: "a failed boot-image construction can publish its partial stage over the complete image",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::tests::a_failed_authoritative_image_stage_preserves_the_complete_image",
        },
    },
    Mutation {
        id: 1207,
        file: "xtask/src/sha256.rs",
        find: r#"            return Ok((hasher.bytes, hasher.finish()));"#,
        replace: r#"            return Ok((hasher.bytes.saturating_sub(1), hasher.finish()));"#,
        breaks: "the streaming SHA-256 reader returns a byte count unrelated to what it hashed",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "sha256::tests::a_reader_is_hashed_without_changing_length_or_digest",
        },
    },
    Mutation {
        id: 1237,
        file: "xtask/src/supply_chain.rs",
        find: r#"    let staging = directory.join(SPDX_STAGING);"#,
        replace: r#"    let staging = directory.join(SPDX_STAGING);
    std::fs::write(destination, b"partial")?;"#,
        breaks: "a failed SPDX stage can expose partial bytes through the authoritative destination",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::a_failed_spdx_stage_never_exposes_partial_authoritative_bytes",
        },
    },
    Mutation {
        id: 1303,
        file: "xtask/src/image/program.rs",
        find: "    let slot = code.get_mut(site).with_context(|| {",
        replace: "    let slot = code.get_mut(0).with_context(|| {",
        breaks: "the fixed witness accepts a branch patch outside its emitted program",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::program::tests::branch_patching_refuses_an_invalid_site",
        },
    },
    Mutation {
        id: 1304,
        file: "xtask/src/image/program.rs",
        find: "        i8::try_from(distance).with_context(|| {",
        replace: "        i8::try_from(distance.min(i8::MAX as usize)).with_context(|| {",
        breaks: "the fixed witness truncates an out-of-range forward branch",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::program::tests::branch_patching_enforces_the_signed_byte_extent",
        },
    },
    Mutation {
        id: 1381,
        file: "xtask/src/cargo.rs",
        find: r#"            Prefix::VerbatimUNC(server, share) => {
                let mut root = OsString::from(r"\\");
                root.push(server);
                root.push(r"\");
                root.push(share);
                PathBuf::from(root)
            }"#,
        replace: "            Prefix::VerbatimUNC(_, _) => return path.to_path_buf(),",
        breaks: "a Windows verbatim UNC target path reaches a GNU toolchain subprocess",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cargo::tests::a_cargo_command_sheds_the_windows_verbatim_unc_target_prefix",
        },
    },
    Mutation {
        id: 1499,
        file: "xtask/src/vendor.rs",
        find: "    let mut file = crate::mutate::workspace::open_real_file(path, \"vendor checksum listing\")?;",
        replace: "    let mut file = std::fs::File::open(path)?;",
        breaks: "a checked vendor listing is reopened through a following open",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::sources::a_checked_path_is_opened_only_without_following_links",
        },
    },
    Mutation {
        id: 1503,
        file: "xtask/src/supply_chain.rs",
        find: "                    source == CRATES_IO,",
        replace: "                    source.starts_with(\"registry+\"),",
        breaks: "an alternate registry's package passes as crates.io",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::a_locked_package_from_any_other_source_is_refused",
        },
    },
    Mutation {
        id: 1504,
        file: "xtask/src/supply_chain.rs",
        find: "                !header.starts_with(\"dependencies.\")\n                    && !(header.starts_with(\"target.\") && header.contains(\".dependencies.\")),",
        replace: "                !(header.starts_with(\"target.\") && header.contains(\".dependencies.\")),",
        breaks: "a detailed dependency table silently leaves the runtime scope",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::every_dependency_declaration_shape_is_modeled_or_refused",
        },
    },
    Mutation {
        id: 1505,
        file: "xtask/src/supply_chain.rs",
        find: "                !header.starts_with(\"dependencies.\")\n                    && !(header.starts_with(\"target.\") && header.contains(\".dependencies.\")),",
        replace: "                !header.starts_with(\"dependencies.\"),",
        breaks: "a target-specific detailed dependency table silently leaves the runtime scope",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::every_dependency_declaration_shape_is_modeled_or_refused",
        },
    },
    Mutation {
        id: 1506,
        file: "xtask/src/supply_chain.rs",
        find: "                !name.contains('.'),",
        replace: "                true,",
        breaks: "a dotted dependency key is read as a dependency name",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::every_dependency_declaration_shape_is_modeled_or_refused",
        },
    },
    Mutation {
        id: 1507,
        file: "xtask/src/supply_chain.rs",
        find: "                !renames_package(declaration),",
        replace: "                true,",
        breaks: "a renamed dependency resolves by its local alias",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::every_dependency_declaration_shape_is_modeled_or_refused",
        },
    },
    Mutation {
        id: 1508,
        file: "xtask/src/supply_chain.rs",
        find: "        noticed == runtime,",
        replace: "        runtime.is_subset(&noticed),",
        breaks: "a notice may list components the image does not ship",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::each_runtime_component_has_exactly_one_exact_notice_entry",
        },
    },
    Mutation {
        id: 1509,
        file: "xtask/src/supply_chain.rs",
        find: "        ensure!(\n            components.insert((name.to_string(), version.to_string())),\n            \"third-party notices repeat `{name}` `{version}`\"\n        );",
        replace: "        components.insert((name.to_string(), version.to_string()));",
        breaks: "a notice may repeat one component's entry",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::each_runtime_component_has_exactly_one_exact_notice_entry",
        },
    },
    Mutation {
        id: 1571,
        file: "xtask/templates/bedrock.vmx.in",
        find: "vvtd.enable = \"TRUE\"\n",
        replace: "",
        breaks: "the VMware machine presents the remapping unit storage requires",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::the_vmx_boots_efi_and_logs_serial_to_a_file",
        },
    },
    Mutation {
        id: 1585,
        file: "xtask/src/supply_chain.rs",
        find: "            ensure!(\n                canonical_header(line),\n                \"`{line}` is a table header spelling the runtime-scope reader does not model\"\n            );\n",
        replace: "",
        breaks: "a dependency table header spelled another way is skipped rather than refused",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::every_dependency_declaration_shape_is_modeled_or_refused",
        },
    },
    Mutation {
        id: 1586,
        file: "xtask/src/supply_chain.rs",
        find: "            unclaimed.remove(&package.name),",
        replace: "            !package.name.is_empty(),",
        breaks: "a path crate outside the workspace is locked without an inventory row",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::a_locked_path_package_must_be_a_distinct_workspace_member",
        },
    },
    Mutation {
        id: 1587,
        file: "xtask/src/supply_chain.rs",
        find: "            unclaimed.remove(&package.name),",
        replace: "            members.contains_key(&package.name),",
        breaks: "a path crate locked under a workspace member's name escapes the inventory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::a_locked_path_package_must_be_a_distinct_workspace_member",
        },
    },
    Mutation {
        id: 1588,
        file: "xtask/src/supply_chain.rs",
        find: "        noticed == runtime,",
        replace: "        noticed.is_subset(&runtime),",
        breaks: "a notice may omit a component the image ships",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::each_runtime_component_has_exactly_one_exact_notice_entry",
        },
    },
    Mutation {
        id: 1597,
        file: "xtask/src/supply_chain.rs",
        find: "                canonical_outer_assignment(line),",
        replace: "                true,",
        breaks: "runtime dependency tables can hide behind outer manifest assignments",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::runtime_dependency_tables_cannot_hide_in_outer_assignments",
        },
    },
    Mutation {
        id: 1632,
        file: "xtask/src/elf.rs",
        find: "        count <= MAX_SECTION_COUNT,",
        replace: "        count <= u16::MAX as usize,",
        breaks: "a kernel ELF can allocate its full hostile section-header cardinality",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "elf::tests::section_count_and_owned_name_bytes_are_bounded_before_amplification",
        },
    },
    Mutation {
        id: 1633,
        file: "xtask/src/elf.rs",
        find: "        total <= MAX_SECTION_NAME_BYTES,",
        replace: "        total <= usize::MAX,",
        breaks: "repeated ELF section-name offsets can amplify owned host memory without a budget",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "elf::tests::section_count_and_owned_name_bytes_are_bounded_before_amplification",
        },
    },
    Mutation {
        id: 1634,
        file: "xtask/src/elf.rs",
        find: "        length <= MAX_KERNEL_ELF_BYTES,",
        replace: "        length <= u64::MAX,",
        breaks: "an oversized kernel ELF reaches host allocation before its shared ceiling refuses it",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "elf::tests::a_kernel_file_over_the_shared_ceiling_is_refused_before_allocation",
        },
    },
    Mutation {
        id: 1641,
        file: "xtask/src/elf.rs",
        find: "    let names = bytes\n        .get(names_at..names_end)\n        .context(\"section name table runs past the end of the file\")?;",
        replace: "    let names = bytes\n        .get(names_at..)\n        .context(\"section name table runs past the end of the file\")?;",
        breaks: "section-name scanning escapes the ELF string table's declared extent",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "elf::tests::section_count_and_owned_name_bytes_are_bounded_before_amplification",
        },
    },
    Mutation {
        id: 1644,
        file: "xtask/src/elf.rs",
        find: "        total_name_bytes = checked_name_budget(total_name_bytes, name.len())?;",
        replace: "        let _ = checked_name_budget(total_name_bytes, name.len())?;",
        breaks: "the ELF parser checks each name against a total it never advances",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "elf::tests::section_count_and_owned_name_bytes_are_bounded_before_amplification",
        },
    },
    Mutation {
        id: 1769,
        file: "xtask/src/image/esp.rs",
        find: "    crate::qualification::read_bounded_file(source, maximum, label)\n        .with_context(|| format!(\"reading {label} {}\", source.display()))",
        replace: "    std::fs::read(source)\n        .with_context(|| format!(\"reading {label} {}\", source.display()))",
        breaks: "boot-image assembly follows and reads an unbounded repository source",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::esp::tests::redirected_image_sources_are_refused_before_copy",
        },
    },
    Mutation {
        id: 1770,
        file: "xtask/src/image/esp.rs",
        find: "    crate::vendor::verify_payload_bytes(&paths.limine_checksums(), &source, &data)?;",
        replace: "",
        breaks: "boot-image assembly embeds a bootloader without its pinned digest",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::esp::tests::image_population_refuses_a_bootloader_that_misses_its_pinned_digest",
        },
    },
    Mutation {
        id: 1786,
        file: "xtask/src/cargo.rs",
        find: "        (source_root, VIRTUAL_SOURCE_ROOT),\n",
        replace: "",
        breaks: "release DWARF retains the checkout's absolute source path",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cargo::tests::kernel_builds_use_the_exact_codegen_and_virtual_path_contract",
        },
    },
    Mutation {
        id: 1789,
        file: "xtask/src/cargo.rs",
        find: "    .env(\"RUSTC\", executable(\"rustc\"))\n",
        replace: "",
        breaks: "a dependency-local toolchain file can select another compiler for a kernel graph",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cargo::tests::kernel_builds_use_the_exact_codegen_and_virtual_path_contract",
        },
    },
    Mutation {
        id: 1831,
        file: "xtask/templates/bedrock.vmx.in",
        find: "acpi.addtable.filename = \"{{DMA_HANDOFF_FILE}}\"",
        replace: "acpi.addtable.filename = \"missing.bin\"",
        breaks: "the VMware machine does not load its generated DMA handoff authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::the_vmx_boots_efi_and_logs_serial_to_a_file",
        },
    },
    Mutation {
        id: 1833,
        file: "xtask/src/platform_acpi.rs",
        find: "    table[0..4].copy_from_slice(&Sdt::DMA_HANDOFF);",
        replace: "    table[0..4].copy_from_slice(&Sdt::DMA_COVERAGE);",
        breaks: "the shared platform table is generated under another ACPI signature",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "platform_acpi::tests::the_shared_dma_handoff_table_carries_the_complete_contract",
        },
    },
    Mutation {
        id: 2060,
        file: "xtask/src/image.rs",
        find: "        let signature = crate::qemu::SELFTEST_IMAGE_MARKER.as_bytes();",
        replace: "        let signature = crate::qemu::SELFTEST_VERDICT_PREFIX.as_bytes();",
        breaks: "an optimizer-split verdict fragment can pose as self-test image identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::tests::a_formatted_verdict_fragment_cannot_pose_as_a_selftest_image",
        },
    },
    Mutation {
        id: 2470,
        file: "xtask/src/cargo.rs",
        find: "        ensure!(\n            path.is_file(),\n            \"the pinned toolchain has no {compiler} at {}; install the toolchain \\",
        replace: "        ensure!(\n            true,\n            \"the pinned toolchain has no {compiler} at {}; install the toolchain \\",
        breaks: "a kernel build whose pinned toolchain lacks rustc or rustdoc reaches cargo",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cargo::tests::a_missing_pinned_compiler_is_named_before_cargo_starts",
        },
    },
];
