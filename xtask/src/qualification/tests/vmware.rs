use super::*;

#[test]
fn vmware_capture_preflights_image_size_before_streaming() {
    let root = crate::test_support::reserve_temp_directory("wizard-streamed-artifact");
    let source = root.join("source.img");
    let captured = root.join("captured.img");
    File::create(&source).unwrap().set_len(4097).unwrap();

    assert!(
        capture_vmware_file(&source, &captured, VMWARE_IMAGE, 4096, 1).is_err(),
        "an oversized image reached the capture stream"
    );
    assert!(
        !captured.exists(),
        "an oversized image created partial qualification bytes"
    );

    OpenOptions::open(OpenOptions::write(&mut File::options(), true), &source)
        .unwrap()
        .set_len(4096)
        .unwrap();
    let digest = capture_vmware_file(&source, &captured, VMWARE_IMAGE, 4096, 1).unwrap();
    assert_eq!(digest.bytes, 4096);
    assert_eq!(std::fs::metadata(captured).unwrap().len(), 4096);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn vmware_capture_refuses_growth_after_its_preflight_measurement() {
    let root = crate::test_support::reserve_temp_directory("wizard-growing-vmware-capture");
    let source = root.join("source.img");
    let captured = root.join("captured.img");
    File::create(&source).unwrap().set_len(4096).unwrap();
    let growing = source.clone();

    let error = capture_vmware_file_with(&source, &captured, VMWARE_IMAGE, 4096, 1, move || {
        OpenOptions::open(OpenOptions::write(&mut OpenOptions::new(), true), growing)?
            .set_len(4097)?;
        Ok(())
    })
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("grew beyond its 4096-byte capture limit while being copied"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn vmware_capture_refuses_truncation_after_exact_preflight() {
    let root = crate::test_support::reserve_temp_directory("wizard-truncated-vmware-capture");
    let source = root.join("source.img");
    let captured = root.join("captured.img");
    File::create(&source).unwrap().set_len(4096).unwrap();
    let shrinking = source.clone();

    let error = capture_vmware_file_with(&source, &captured, VMWARE_IMAGE, 4096, 1, move || {
        OpenOptions::open(OpenOptions::write(&mut OpenOptions::new(), true), shrinking)?
            .set_len(4095)?;
        Ok(())
    })
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("changed outside its exact 4096-byte capture extent"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn generation_receipt_binds_reset_source_selector_and_machine_bytes() {
    let directory = crate::test_support::reserve_temp_directory("wizard-vmware-receipt");
    for name in [
        VMWARE_IMAGE,
        VMWARE_NAMESPACE,
        VMWARE_VMDK,
        VMWARE_NAMESPACE_VMDK,
        VMWARE_VMX,
    ] {
        std::fs::write(directory.join(name), name).unwrap();
    }
    std::fs::write(
        directory.join(VMWARE_HANDOFF),
        crate::platform_acpi::dma_handoff_table(),
    )
    .unwrap();
    let generation = "1".repeat(64);
    std::fs::write(
        directory.join(VMWARE_VMX),
        crate::vmware::generation_vmx(
            Kernel::SelfTest,
            &generation_transcript_name(&generation).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let transcript = directory.join(generation_transcript_name(&generation).unwrap());
    std::fs::write(&transcript, []).unwrap();
    require_empty_reset(&digest_file(&transcript).unwrap()).unwrap();
    require_generation_inventory(&directory, &transcript, false).unwrap();
    std::fs::write(directory.join("nvram"), "stale firmware state").unwrap();
    assert!(require_generation_inventory(&directory, &transcript, false).is_err());
    std::fs::remove_file(directory.join("nvram")).unwrap();
    std::fs::remove_file(directory.join(VMWARE_IMAGE)).unwrap();
    std::fs::create_dir(directory.join(VMWARE_IMAGE)).unwrap();
    assert!(require_generation_inventory(&directory, &transcript, false).is_err());
    std::fs::remove_dir(directory.join(VMWARE_IMAGE)).unwrap();
    std::fs::write(directory.join(VMWARE_IMAGE), VMWARE_IMAGE).unwrap();
    let source = SourceIdentity {
        commit: "a".repeat(40),
        tree: "b".repeat(40),
    };
    let record = generation_record(&source, Kind::VmwareSelfTest, &directory, &generation).unwrap();
    std::fs::write(directory.join(VMWARE_GENERATION_RECEIPT), record.render()).unwrap();
    assert!(
        verify_generation_with_source(&source, Kind::VmwareSelfTest, &directory, &transcript)
            .is_err(),
        "an untouched reset transcript counted as execution"
    );

    std::fs::write(&transcript, "fresh machine output\n").unwrap();
    assert!(require_empty_reset(&digest_file(&transcript).unwrap()).is_err());
    verify_generation_with_source(&source, Kind::VmwareSelfTest, &directory, &transcript).unwrap();
    let stale = SourceIdentity {
        commit: "9".repeat(40),
        tree: source.tree.clone(),
    };
    assert!(
        verify_generation_with_source(&stale, Kind::VmwareSelfTest, &directory, &transcript)
            .is_err()
    );
    assert!(
        verify_generation_with_source(&source, Kind::VmwareShipped, &directory, &transcript)
            .is_err()
    );
    std::fs::write(directory.join(VMWARE_VMX), "changed").unwrap();
    assert!(
        verify_generation_with_source(&source, Kind::VmwareSelfTest, &directory, &transcript)
            .is_err()
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn vmware_accepts_only_the_source_defined_dma_handoff_authority() {
    let root = crate::test_support::reserve_temp_directory("wizard-vmware-handoff");
    let path = root.join("handoff.bin");
    let mut foreign = crate::platform_acpi::dma_handoff_table();
    foreign[10] ^= 1;
    foreign[9] = 0;
    foreign[9] = 0u8.wrapping_sub(
        foreign
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte)),
    );
    std::fs::write(&path, foreign).unwrap();
    assert!(
        acpi_tables::DmaHandoff::new(&foreign).is_ok(),
        "the near miss must remain structurally valid ACPI"
    );
    assert!(verified_dma_handoff_digest(&path).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn one_immutable_vmware_generation_is_captured_before_publication() {
    fn write_generation(
        directory: &Path,
        source: &SourceIdentity,
        generation: &str,
        tag: &str,
    ) -> PathBuf {
        std::fs::create_dir_all(directory).unwrap();
        for name in [
            VMWARE_IMAGE,
            VMWARE_NAMESPACE,
            VMWARE_VMDK,
            VMWARE_NAMESPACE_VMDK,
            VMWARE_VMX,
        ] {
            std::fs::write(directory.join(name), format!("{tag}:{name}")).unwrap();
        }
        std::fs::write(
            directory.join(VMWARE_HANDOFF),
            crate::platform_acpi::dma_handoff_table(),
        )
        .unwrap();
        std::fs::write(
            directory.join(VMWARE_VMX),
            crate::vmware::generation_vmx(
                Kernel::SelfTest,
                &generation_transcript_name(generation).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let transcript = directory.join(generation_transcript_name(generation).unwrap());
        std::fs::write(&transcript, format!("{tag}:transcript\n")).unwrap();
        let receipt =
            generation_record(source, Kind::VmwareSelfTest, directory, generation).unwrap();
        std::fs::write(directory.join(VMWARE_GENERATION_RECEIPT), receipt.render()).unwrap();
        transcript
    }

    fn capture(directory: &Path, transcript: &Path, captured: &Path) -> CapturedVmwareGeneration {
        let image = directory.join(VMWARE_IMAGE);
        let namespace = directory.join(VMWARE_NAMESPACE);
        std::fs::create_dir(captured).unwrap();
        CapturedVmwareGeneration::read_with_sizes(
            &VmwareArtifacts {
                image: &directory.join(VMWARE_IMAGE),
                namespace: &directory.join(VMWARE_NAMESPACE),
                vmdk: &directory.join(VMWARE_VMDK),
                namespace_vmdk: &directory.join(VMWARE_NAMESPACE_VMDK),
                vmx: &directory.join(VMWARE_VMX),
                handoff: &directory.join(VMWARE_HANDOFF),
                transcript,
                receipt: &directory.join(VMWARE_GENERATION_RECEIPT),
            },
            captured,
            std::fs::metadata(image).unwrap().len(),
            std::fs::metadata(namespace).unwrap().len(),
        )
        .unwrap()
    }

    let root = crate::test_support::reserve_temp_directory("wizard-vmware-capture");
    let source = SourceIdentity {
        commit: "a".repeat(40),
        tree: "b".repeat(40),
    };
    let generation_a = "1".repeat(64);
    let generation_b = "2".repeat(64);
    let directory_a = root.join("generation-a");
    let directory_b = root.join("generation-b");
    let transcript_a = write_generation(&directory_a, &source, &generation_a, "generation-a");
    let transcript_b = write_generation(&directory_b, &source, &generation_b, "generation-b");

    let generated_vmx = std::fs::read(directory_a.join(VMWARE_VMX)).unwrap();
    let image_a = std::fs::read(directory_a.join(VMWARE_IMAGE)).unwrap();
    let image_b = std::fs::read(directory_b.join(VMWARE_IMAGE)).unwrap();
    let mut runtime_vmx = generated_vmx.clone();
    runtime_vmx.extend_from_slice(b"cleanShutdown = \"FALSE\"\n");
    std::fs::write(directory_a.join(VMWARE_VMX), &runtime_vmx).unwrap();

    let valid = capture(&directory_a, &transcript_a, &root.join("captured-valid"));
    valid
        .validate_generation(&source, Kind::VmwareSelfTest)
        .unwrap();
    assert_ne!(
        valid.file(VMWARE_VMX).unwrap().digest,
        digest_bytes(&generated_vmx)
    );
    verify_generation_with_source(&source, Kind::VmwareSelfTest, &directory_a, &transcript_a)
        .unwrap();
    verify_generation_with_source(
        &source,
        Kind::VmwareSelfTest,
        &root.join("captured-valid"),
        &root.join("captured-valid").join(VMWARE_TRANSCRIPT),
    )
    .unwrap();

    std::fs::copy(directory_b.join(VMWARE_VMX), directory_a.join(VMWARE_VMX)).unwrap();
    let mixed = capture(&directory_a, &transcript_a, &root.join("captured-mixed"));
    assert!(
        mixed
            .validate_generation(&source, Kind::VmwareSelfTest)
            .is_err(),
        "machine bytes from two valid generations were combined"
    );

    std::fs::write(directory_a.join(VMWARE_VMX), &runtime_vmx).unwrap();
    let wrong_transcript = capture(
        &directory_a,
        &transcript_b,
        &root.join("captured-wrong-transcript"),
    );
    assert!(
        wrong_transcript
            .validate_generation(&source, Kind::VmwareSelfTest)
            .is_err(),
        "a valid transcript path from another generation was accepted"
    );

    std::fs::write(directory_a.join(VMWARE_IMAGE), &image_b).unwrap();
    let mixed_receipt = capture(
        &directory_a,
        &transcript_a,
        &root.join("captured-mixed-receipt"),
    );
    assert!(
        mixed_receipt
            .validate_generation(&source, Kind::VmwareSelfTest)
            .is_err(),
        "machine bytes from two valid generations shared one receipt"
    );
    std::fs::write(directory_a.join(VMWARE_IMAGE), image_a).unwrap();

    let published = root.join("published");
    let captured = capture(&directory_a, &transcript_a, &published);
    captured
        .validate_generation(&source, Kind::VmwareSelfTest)
        .unwrap();
    let captured_vmx = std::fs::read(&captured.file(VMWARE_VMX).unwrap().captured).unwrap();
    std::fs::write(directory_a.join(VMWARE_VMX), b"changed after capture").unwrap();
    let mut record = Record::default();
    captured.publish(&published, &mut record).unwrap();
    assert_eq!(
        std::fs::read(published.join(VMWARE_VMX)).unwrap(),
        captured_vmx
    );
    assert!(captured.verify_sources().is_err());
    assert!(
        !published.join(RECORD).exists(),
        "a changed capture published an authoritative qualification record"
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn each_vmware_generation_owns_a_distinct_serial_file() {
    let first = fresh_vmware_transcript_name().unwrap();
    let second = fresh_vmware_transcript_name().unwrap();
    assert_ne!(first, second);
    for name in [first, second] {
        assert!(name.starts_with(SERIAL_PREFIX));
        assert!(name.ends_with(SERIAL_SUFFIX));
        assert!(is_digest(
            name.strip_prefix(SERIAL_PREFIX)
                .and_then(|name| name.strip_suffix(SERIAL_SUFFIX))
                .unwrap()
        ));
    }
}

#[test]
fn a_vmware_selector_directory_cannot_be_substituted() {
    let root = crate::test_support::reserve_temp_directory("wizard-vmware-selector");
    let repository = root.join("repository");
    let vm_root = repository.join("vm");
    let selector = vm_root.join("self-test");
    let alternate = vm_root.join("alternate");
    let outside = root.join("outside");
    std::fs::create_dir_all(&selector).unwrap();
    std::fs::create_dir(&alternate).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let paths = Paths::new(repository, None);
    require_exact_vmware_directory(&paths, Kind::VmwareSelfTest, &selector).unwrap();

    std::fs::remove_dir(&selector).unwrap();
    create_directory_symlink(&alternate, &selector);
    assert!(
        require_exact_vmware_directory(&paths, Kind::VmwareSelfTest, &selector).is_err(),
        "an authoritative selector name accepted another in-root directory"
    );
    crate::test_support::remove_directory_link(&selector);
    create_directory_symlink(&outside, &selector);
    assert!(
        require_exact_vmware_directory(&paths, Kind::VmwareSelfTest, &selector).is_err(),
        "an authoritative selector name escaped the VMware root"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn vmrun_list_requires_one_exact_running_machine() {
    let root = crate::test_support::reserve_temp_directory("wizard-vmrun-list");
    let expected = root.join("expected.vmx");
    let foreign = root.join("foreign.vmx");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&expected, "vmx").unwrap();
    std::fs::write(&foreign, "vmx").unwrap();
    let valid = format!("{VMRUN_LIST_HEADER}1\n{}", expected.display());
    let observed = parse_vmrun_list(&valid).unwrap();
    require_observed_vmx(&observed, &expected).unwrap();

    for invalid in [
        format!("{VMRUN_LIST_HEADER}0"),
        format!(
            "{VMRUN_LIST_HEADER}2\n{}\n{}",
            expected.display(),
            foreign.display()
        ),
        format!("{VMRUN_LIST_HEADER}1"),
        format!("{VMRUN_LIST_HEADER}01\n{}", expected.display()),
        format!("Running VMs: 1\n{}", expected.display()),
        format!("{VMRUN_LIST_HEADER}1\n{}\nextra", expected.display()),
        format!("{VMRUN_LIST_HEADER}1\r\n{}", expected.display()),
    ] {
        assert!(parse_vmrun_list(&invalid).is_err(), "accepted {invalid:?}");
    }
    assert!(require_observed_vmx(&foreign, &expected).is_err());
    assert!(require_observed_vmx(Path::new("expected.vmx"), &expected).is_err());
    assert_eq!(VMRUN_LIST_ARGS, ["-T", "ws", "list"]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn offline_vmware_observer_is_bound_to_its_exact_selector() {
    let root = crate::test_support::reserve_temp_directory("wizard-vmrun-replay");
    let repository = root.join("repository");
    let qualification = root.join("qualification");
    let paths = Paths::new(repository.clone(), Some(&root.join("target")));
    let self_test_vmx = repository.join("vm/self-test").join(VMWARE_VMX);
    let shipped_vmx = repository.join("vm/shipped").join(VMWARE_VMX);
    let foreign_vmx = root.join("foreign.vmx");
    for path in [&self_test_vmx, &shipped_vmx, &foreign_vmx] {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "vmx").unwrap();
    }
    for kind in [Kind::VmwareSelfTest, Kind::VmwareShipped] {
        std::fs::create_dir_all(qualification.join(kind.slug())).unwrap();
    }

    let write_observer = |kind: Kind, vmx: &Path| {
        let observer = qualification.join(kind.slug()).join(VMWARE_OBSERVER);
        std::fs::write(
            &observer,
            format!("{VMRUN_LIST_HEADER}1\n{}\n", vmx.display()),
        )
        .unwrap();
        let mut record = Record::default();
        add_artifact(&mut record, "artifact.observer", VMWARE_OBSERVER, &observer).unwrap();
        record
    };

    let self_test = write_observer(Kind::VmwareSelfTest, &self_test_vmx);
    verify_vmware_observer(&paths, &self_test, Kind::VmwareSelfTest, &qualification).unwrap();
    let shipped = write_observer(Kind::VmwareShipped, &shipped_vmx);
    verify_vmware_observer(&paths, &shipped, Kind::VmwareShipped, &qualification).unwrap();

    let swapped = write_observer(Kind::VmwareSelfTest, &shipped_vmx);
    assert!(
        verify_vmware_observer(&paths, &swapped, Kind::VmwareSelfTest, &qualification,).is_err(),
        "a coherently checksummed shipped observer qualified the self-test selector"
    );
    let foreign = write_observer(Kind::VmwareSelfTest, &foreign_vmx);
    assert!(
        verify_vmware_observer(&paths, &foreign, Kind::VmwareSelfTest, &qualification,).is_err(),
        "a coherently checksummed foreign absolute VMX qualified"
    );

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn vmrun_observation_executes_list_between_exact_tool_probes() {
    let root = crate::test_support::reserve_temp_directory("wizard-vmrun-order");
    let expected = root.join("expected.vmx");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&expected, "vmx").unwrap();
    let tool = Tool {
        path: root.join("vmrun"),
        version: "vmrun version test build-1".to_string(),
    };
    let observer = format!("{VMRUN_LIST_HEADER}1\n{}", expected.display());
    let mut calls = Vec::new();

    let actual = observe_running_vm_with(&tool, &expected, |program, args, policy| {
        calls.push((
            program.to_path_buf(),
            args.iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>(),
            policy,
        ));
        match args {
            [] => Ok(tool.version.clone()),
            ["-T", "ws", "list"] => Ok(observer.clone()),
            _ => bail!("unexpected vmrun arguments: {args:?}"),
        }
    })
    .unwrap();

    assert_eq!(actual, observer);
    assert_eq!(
        calls,
        [
            (tool.path.clone(), Vec::new(), ExitPolicy::Any),
            (
                tool.path.clone(),
                vec!["-T".to_string(), "ws".to_string(), "list".to_string()],
                ExitPolicy::Success,
            ),
            (tool.path.clone(), Vec::new(), ExitPolicy::Any),
        ]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn generation_start_invalidates_both_receipt_names() {
    let directory = crate::test_support::reserve_temp_directory("wizard-generation-invalidation");
    let receipt = directory.join(VMWARE_GENERATION_RECEIPT);
    std::fs::write(&receipt, "stale pass").unwrap();
    std::fs::write(directory.join(VMWARE_GENERATION_TEMP), "partial pass").unwrap();

    invalidate_vmware_generation(&receipt).unwrap();

    assert!(!receipt.exists());
    assert!(!directory.join(VMWARE_GENERATION_TEMP).exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn vmrun_version_is_separated_from_its_usage_text() {
    let report = "\r\nvmrun version 1.17.0 build-24583834\r\n\r\nUsage: vmrun COMMAND\r\n";
    assert_eq!(
        vmrun_version(report).unwrap(),
        "vmrun version 1.17.0 build-24583834"
    );
    assert!(vmrun_version("Usage: vmrun COMMAND").is_err());
}
