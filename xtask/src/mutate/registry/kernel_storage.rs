use super::{Caught, Mutation};

pub(super) const MUTATIONS: &[Mutation] = &[
    Mutation {
        id: 17,
        file: "kernel/src/nvme/preparation.rs",
        find: "    match disabled.access(|registers| write(registers, offset, value)) {",
        replace: "    match disabled.access(|registers| {\n        #[cfg(debug_assertions)]\n        let stack_pressure = [0u8; 3072];\n        #[cfg(not(debug_assertions))]\n        let stack_pressure = [0u8; 16 * 1024];\n        core::hint::black_box(&stack_pressure);\n        let result = write(registers, offset, value);\n        core::hint::black_box(&stack_pressure);\n        result\n    }) {",
        breaks: "NVMe preparation consumes the two-page boot-stack safety margin",
        reddens: Caught::Check("nvme candidates preserve two pages of boot-stack headroom"),
    },
    Mutation {
        id: 55,
        file: "kernel/src/vfs.rs",
        find: "            let listing = crate::tmpfs::list(index).ok_or(VfsError::NotFound)?;\n            for entry in listing.entries() {",
        replace: "            let listing = crate::tmpfs::list(index).ok_or(VfsError::NotFound)?;\n            static REGRESSION: wost::sync::SpinLock<()> = wost::sync::SpinLock::new(());\n            let _backend = REGRESSION.lock_leaf();\n            for entry in listing.entries() {",
        breaks: "a directory callback runs while its backend still owns a leaf lock",
        reddens: Caught::Check("enumeration callbacks run without backend ownership"),
    },
    Mutation {
        id: 56,
        file: "kernel/src/nvme.rs",
        find: "        match device.admit_bus_master(authority, domain, interrupts) {",
        replace: "        match device\n            .admit_bus_master(authority, domain, interrupts)\n            .or(Ok::<(), pci_config::EnableError>(()))\n        {",
        breaks: "a refused bus-master transition mints start authority",
        reddens: Caught::Check("a failed bus master admission stops before nvme start"),
    },
    Mutation {
        id: 186,
        file: "kernel/src/devfs.rs",
        find: "pub(crate) const DEVICES: [Device; 2] = [Device::Storage, Device::Console];",
        replace: "pub(crate) const DEVICES: [Device; 1] = [Device::Storage];",
        breaks: "a device variant is absent from the devfs listing",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::kernel_source::the_devfs_source_inventory_matches_every_variant",
        },
    },
    Mutation {
        id: 199,
        file: "kernel/src/object.rs",
        find: "            Self::Channel => Rights::NONE,",
        replace: "            Self::Channel => Rights::READ,",
        breaks: "a bootstrap identity admits no speculative channel data rights",
        reddens: Caught::Check("a meaningless right is refused"),
    },
    Mutation {
        id: 200,
        file: "kernel/src/object.rs",
        find: "            Self::File => Rights::READ,",
        replace: "            Self::File => Rights::READ.union(Rights::WRITE),",
        breaks: "a file refuses a right no operation consults",
        reddens: Caught::Check("a meaningless right is refused"),
    },
    Mutation {
        id: 201,
        file: "kernel/src/object.rs",
        find: "            Self::Directory => Rights::TRAVERSE.union(Rights::ENUMERATE),",
        replace: "            Self::Directory => Rights::TRAVERSE
                .union(Rights::ENUMERATE)
                .union(Rights::MODIFY),",
        breaks: "a directory refuses a right no operation consults",
        reddens: Caught::Check("a meaningless right is refused"),
    },
    // Replace the controller-bound validation with one known-good request;
    // the submission interceptor catches it before any doorbell.
    Mutation {
        id: 247,
        file: "kernel/src/nvme/io.rs",
        find: "        let request = self.validate_read(lba, blocks, buffer_bytes)?;",
        replace: "        let request = self\n            .validate_read(lba, blocks, buffer_bytes)\n            .or_else(|_| {\n                ReadTransfer::new(\n                    0,\n                    1,\n                    self.blocks,\n                    u64::from(self.block_bytes),\n                    u64::MAX,\n                )\n                .map_err(BringUpError::InvalidTransfer)\n            })?;",
        breaks: "raw nvme reads cannot bypass controller-bound validation",
        reddens: Caught::Check("invalid nvme reads stop before submission"),
    },
    // Replace refused device data with a known-good synthetic namespace. The
    // self-test queue interceptor catches the bypass before allocation or MMIO.
    Mutation {
        id: 257,
        file: "kernel/src/nvme/io.rs",
        find: "        let namespace = Namespace::parse(identify).map_err(BringUpError::InvalidNamespace)?;",
        replace: "        let namespace = Namespace::parse(identify)\n            .or_else(|_| {\n                let mut accepted = [0u8; NAMESPACE_IDENTIFY_BYTES];\n                accepted[0] = 1;\n                accepted[128 + 2] = 9;\n                Namespace::parse(&accepted)\n            })\n            .map_err(BringUpError::InvalidNamespace)?;",
        breaks: "invalid namespace data cannot bypass io queue admission",
        reddens: Caught::Check("invalid nvme namespace stops before io queues"),
    },
    // The phase tag never flips: fine until the queue wraps, so the check
    // must issue more commands than the queue has entries.
    Mutation {
        id: 260,
        file: "kernel/src/nvme/io.rs",
        find: "                if completion_head == 0 {
                    queue.phase = !queue.phase;
                }",
        replace: "                if false {
                    queue.phase = !queue.phase;
                }",
        breaks: "a completion queue wraps and stays readable",
        reddens: Caught::Check("the queues wrap and the phase tag still works"),
    },
    // The volume mounted at sector zero, which is where a reader that could
    // not be given an offset would look.
    Mutation {
        id: 265,
        file: "kernel/src/fs.rs",
        find: "pub(crate) const VOLUME_SECTOR: u64 = 2048;",
        replace: "pub(crate) const VOLUME_SECTOR: u64 = 0;",
        breaks: "the volume is where the partition puts it",
        reddens: Caught::Check("the volume mounts and says what a fat32 volume says"),
    },
    // Globally unique fixture names hide a missing parent comparison, so two
    // fixture directories deliberately contain `MOTD` (D-172).
    Mutation {
        id: 269,
        file: "kernel/src/tmpfs.rs",
        find: "        if index == ROOT || entry.parent != directory {
            continue;
        }
        if &entry.name[..entry.name_len] != name {",
        replace: "        if index == ROOT {
            continue;
        }
        if &entry.name[..entry.name_len] != name {",
        breaks: "a directory holds only its own entries",
        reddens: Caught::Check("a second backend answers the same three operations"),
    },
    // A path walked *through* a device, which has nothing inside it — the
    // "everything is a file" conflation arriving as a defect.
    Mutation {
        id: 270,
        file: "kernel/src/vfs.rs",
        find: "    if rest.is_empty() {
        Ok(Node::Device(device))
    } else {
        Err(VfsError::NotADirectory)
    }",
        replace: "    let _ = rest;
    Ok(Node::Device(device))",
        breaks: "a device is not a directory",
        reddens: Caught::Check("a device is not a file"),
    },
    // A tmpfs read bounded by the arena rather than by the file, which hands
    // back the next file's bytes.
    Mutation {
        id: 271,
        file: "kernel/src/tmpfs.rs",
        find: "    let wanted = into.len().min(len - offset);",
        replace: "    let wanted = into.len();",
        breaks: "a read stops at the end of the file, in memory too",
        reddens: Caught::Check("a second backend answers the same three operations"),
    },
    // Treat an unverified stop as verified; a fake device completes after
    // the allocator could already reuse the command's memory.
    Mutation {
        id: 298,
        file: "kernel/src/nvme.rs",
        find: "            self.disposition = DmaDisposition::Retained;",
        replace: "            self.disposition = DmaDisposition::Quiesced;",
        breaks: "an unverified controller stop retains its dma memory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::unproven_stop_retains_every_reachable_run",
        },
    },
    // Repair follows the red observation so the outstanding command remains observable.
    Mutation {
        id: 300,
        file: "kernel/src/nvme/io.rs",
        find: "            core::hint::spin_loop();\n        }\n        self.fail_submitted(BringUpError::NotReady, data, isolation, data_mapped)",
        replace: "            core::hint::spin_loop();\n        }\n        Err(BringUpError::NotReady)",
        breaks: "a command timeout settles dma before returning",
        reddens: Caught::Check("an nvme timeout quiesces before dma memory returns"),
    },
    // Publish queue addresses before moving them into their cleanup owner.
    Mutation {
        id: 333,
        file: "kernel/src/nvme/preparation.rs",
        find: "    // Cleanup owns every published queue address before the enable write.
    let controller = Controller {",
        replace: "    let _ = write(&mut registers, register::CC, cc);
    let controller = Controller {",
        breaks: "nvme cleanup owns every published queue before controller enable",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::kernel_source::nvme_cleanup_and_bus_master_authority_precede_the_enable_write",
        },
    },
    Mutation {
        id: 532,
        file: "kernel/src/nvme/io.rs",
        find: "        self.read_blocks(0, blocks, into).ok()?;",
        replace: "        self.read_blocks(self.blocks, blocks, into).ok()?;",
        breaks: "the storage baseline never submits inside the namespace",
        reddens: Caught::Check("a full-buffer nvme read is measurable"),
    },
    Mutation {
        id: 574,
        file: "kernel/src/nvme/io.rs",
        find: "            self.stop_dma(Some(data));",
        replace: "            let _ = self.fail_submitted(BringUpError::NotReady, Some(data));",
        breaks: "NVMe self-test repair hides cleanup behind a guaranteed error",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::errors::known_fallible_results_are_not_discarded",
        },
    },
    Mutation {
        id: 934,
        file: "kernel/src/nvme.rs",
        find: "    wanted: Readiness,",
        replace: "    wanted: bool,",
        breaks: "NVMe ready-state polling becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 958,
        file: "kernel/src/nvme.rs",
        find: "    wait_for_ready(registers, *capabilities, Readiness::NotReady)?;",
        replace: "    wait_for_ready(registers, *capabilities, Readiness::Ready)?;",
        breaks: "controller-stop evidence is minted after observing the ready bit set",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 959,
        file: "kernel/src/nvme.rs",
        find: "    let cc = read(registers, register::CC)?;",
        replace: "    let cc = 0;",
        breaks: "controller-stop evidence substitutes a constant for its fresh configuration read",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 960,
        file: "kernel/src/nvme.rs",
        find: "    let csts = read(registers, register::CSTS)?;",
        replace: "    let csts = 0;",
        breaks: "controller-stop evidence substitutes a constant for its fresh status read",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 961,
        file: "kernel/src/nvme.rs",
        find: "    let observed = DisabledRegisters::from_bits(cc, csts).map_err(|_| BringUpError::NotReady)?;",
        replace: "    let observed = DisabledRegisters::from_bits(0, 0).map_err(|_| BringUpError::NotReady)?;",
        breaks: "controller-stop evidence classifies constants instead of its fresh register pair",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 962,
        file: "kernel/src/nvme.rs",
        find: "            self.dma.disposition = DmaDisposition::Quiesced;",
        replace: "            self.dma.disposition = DmaDisposition::Retained;",
        breaks: "proven controller-stop evidence retains recyclable DMA ownership",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::proven_stop_consumes_evidence_before_recycling",
        },
    },
    Mutation {
        id: 963,
        file: "kernel/src/nvme.rs",
        find: "    dma: &'controller mut DmaState,",
        replace: "    dma: &'controller DmaState,",
        breaks: "controller-stop evidence loses mutable authority over the exact DMA state it retires",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 964,
        file: "kernel/src/nvme.rs",
        find: "    fn quiesce(self) {",
        replace: "    fn quiesce(&mut self) {",
        breaks: "one controller-stop observation can authorize more than one DMA recycle transition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::proven_stop_consumes_evidence_before_recycling",
        },
    },
    Mutation {
        id: 965,
        file: "kernel/src/nvme.rs",
        find: r#"    Ok(ControllerStopped {
        _registers: registers,
        _observed: observed,
        dma,
    })
}"#,
        replace: r#"    Ok(mint_stopped(registers, observed, dma))
}

fn mint_stopped<'controller>(
    registers: &'controller MappedBar,
    observed: DisabledRegisters,
    dma: &'controller mut DmaState,
) -> ControllerStopped<'controller> {
    ControllerStopped {
        _registers: registers,
        _observed: observed,
        dma,
    }
}"#,
        breaks: "stop evidence can bind a hardware observation and DMA state from different controllers",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 969,
        file: "kernel/src/nvme.rs",
        find: "#[derive(Debug)]\nstruct ControllerStopped<'controller> {\n    _registers: &'controller MappedBar,\n    _observed: DisabledRegisters,\n    dma: &'controller mut DmaState,\n}",
        replace: "#[derive(Debug)]\nstruct ControllerStopped<'controller> {\n    _registers: &'controller MappedBar,\n    _observed: DisabledRegisters,\n    dma: &'controller mut DmaState,\n}\n\nconst _: () = {\n    impl Clone for ControllerStopped<'_> {\n        fn clone(&self) -> Self {\n            unreachable!()\n        }\n    }\n};",
        breaks: "a nested trait implementation can duplicate one observed controller stop into multiple recycle authorities",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 970,
        file: "kernel/src/nvme.rs",
        find: "#[derive(Debug, PartialEq, Eq)]\nenum DmaDisposition {",
        replace: "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\nenum DmaDisposition {",
        breaks: "a controller's DMA retirement disposition becomes freely copyable",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::dma_disposition_cannot_move_between_controllers",
        },
    },
    Mutation {
        id: 971,
        file: "kernel/src/nvme.rs",
        find: "impl ControllerStopped<'_> {\n    fn quiesce(self) {",
        replace: "impl ControllerStopped<'_> {\n    fn duplicate(self) -> Self {\n        self\n    }\n\n    fn quiesce(self) {",
        breaks: "controller-stop evidence gains an extra inherent transfer path",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 972,
        file: "kernel/src/nvme.rs",
        find: "struct ControllerStopped<'controller> {\n    _registers: &'controller MappedBar,\n    _observed: DisabledRegisters,\n    dma: &'controller mut DmaState,\n}\n\nimpl ControllerStopped<'_> {",
        replace: "struct ControllerStopped<'controller> {\n    _registers: &'controller MappedBar,\n    _observed: DisabledRegisters,\n    dma: &'controller mut DmaState,\n}\n\ntype ControllerStoppedAlias<'controller> = ControllerStopped<'controller>;\n\nimpl ControllerStopped<'_> {",
        breaks: "controller-stop evidence gains a constructor alias",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 973,
        file: "kernel/src/nvme.rs",
        find: "struct ControllerStopped<'controller> {\n    _registers: &'controller MappedBar,\n    _observed: DisabledRegisters,\n    dma: &'controller mut DmaState,\n}\n\nimpl ControllerStopped<'_> {",
        replace: "struct ControllerStopped<'controller> {\n    _registers: &'controller MappedBar,\n    _observed: DisabledRegisters,\n    dma: &'controller mut DmaState,\n}\n\nuse self::ControllerStopped as ForgedStopped;\n\nimpl ControllerStopped<'_> {",
        breaks: "controller-stop evidence gains an imported constructor alias",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 974,
        file: "kernel/src/nvme.rs",
        find: "    const fn is_settled(&self) -> bool {\n        !matches!(&self.disposition, DmaDisposition::Active)\n    }",
        replace: "    const fn is_settled(&self) -> bool {\n        true\n    }",
        breaks: "active DMA is reported settled before hardware quiescence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::dma_disposition_cannot_move_between_controllers",
        },
    },
    Mutation {
        id: 975,
        file: "kernel/src/nvme.rs",
        find: "        if self.dma.disposition == DmaDisposition::Active {",
        replace: "        if true {",
        breaks: "consumed stop evidence can rewrite an already terminal DMA disposition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::proven_stop_consumes_evidence_before_recycling",
        },
    },
    Mutation {
        id: 976,
        file: "kernel/src/nvme/io.rs",
        find: "        if self.dma.must_retain() {\n            self.admin.retain();\n            if let Some(queue) = self.io.as_mut() {\n                queue.retain();\n            }\n            if let Some(segment) = data {\n                segment.retain();\n            }\n        }",
        replace: "",
        breaks: "a failed controller stop retains none of its reachable DMA runs",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::failed_stop_retention_and_retirement_sinks_are_exact",
        },
    },
    Mutation {
        id: 977,
        file: "kernel/src/nvme/queues.rs",
        find: "fn retain(&mut self) {\n        self.submission.retain();\n        self.completion.retain();\n    }",
        replace: "fn retain(&mut self) {\n        self.submission.retain();\n    }",
        breaks: "failed controller cleanup recycles the completion queue while hardware may reach it",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::failed_stop_retention_and_retirement_sinks_are_exact",
        },
    },
    Mutation {
        id: 978,
        file: "kernel/src/nvme/io.rs",
        find: "        if !self.dma.is_quiesced() {\n            return quarantine_controller(self);\n        }",
        replace: "        if !self.dma.is_quiesced() {\n            return ControllerRetirement::Retired;\n        }",
        breaks: "an unverified controller stop drops its complete quarantine owner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::controller_retirement_returns_every_cleanup_owner",
        },
    },
    Mutation {
        id: 979,
        file: "kernel/src/nvme/preparation.rs",
        find: "        mem::MappedPciBarRetirement::Quarantined => PrepareFailure::memory_quarantine(reason),",
        replace: "        mem::MappedPciBarRetirement::Quarantined => PrepareFailure::Retired(reason),",
        breaks: "memory-owned BAR quarantine is misreported as successful retirement",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::mapped_bar_cleanup_quarantine_cannot_be_reported_retired",
        },
    },
    Mutation {
        id: 980,
        file: "kernel/src/nvme.rs",
        find: "    *retained = Some(RetainedController {\n        _controller: controller,\n        device: None,\n    });",
        replace: "    drop(controller);",
        breaks: "controller quarantine mints its token after dropping the controller owner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::controller_retirement_returns_every_cleanup_owner",
        },
    },
    Mutation {
        id: 981,
        file: "kernel/src/nvme.rs",
        find: "impl Drop for Controller {\n    fn drop(&mut self) {\n        self.stop_dma(None);\n    }\n}",
        replace: "impl Drop for Controller {\n    fn drop(&mut self) {}\n}",
        breaks: "dropping a controller skips the final DMA stop-or-retain boundary",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::failed_stop_retention_and_retirement_sinks_are_exact",
        },
    },
    Mutation {
        id: 982,
        file: "kernel/src/nvme.rs",
        find: "        if observed == wanted {",
        replace: "        if observed == wanted || wanted == Readiness::NotReady {",
        breaks: "a disable wait succeeds before observing the controller ready bit clear",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::controller_stop_waits_for_the_requested_hardware_state",
        },
    },
    Mutation {
        id: 983,
        file: "kernel/src/nvme.rs",
        find: "        let observed = if bits & status::READY != 0 {",
        replace: "        let observed = if bits & status::READY == 0 {",
        breaks: "the readiness poll reverses the controller's live ready-bit observation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::controller_stop_waits_for_the_requested_hardware_state",
        },
    },
    Mutation {
        id: 984,
        file: "kernel/src/nvme.rs",
        find: "    const fn must_retain(&self) -> bool {\n        matches!(&self.disposition, DmaDisposition::Retained)\n    }\n}\n\n/// A mapped controller observed disabled while PCI bus mastering was off.",
        replace: "    const fn must_retain(&self) -> bool {\n        matches!(&self.disposition, DmaDisposition::Retained)\n    }\n}\n\nfn swap_dma_states(left: &mut Controller, right: &mut Controller) {\n    core::mem::swap(&mut left.dma, &mut right.dma);\n}\n\n/// A mapped controller observed disabled while PCI bus mastering was off.",
        breaks: "DMA retirement state can be swapped between two controllers",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::dma_disposition_cannot_move_between_controllers",
        },
    },
    Mutation {
        id: 985,
        file: "kernel/src/nvme.rs",
        find: "    const fn must_retain(&self) -> bool {\n        matches!(&self.disposition, DmaDisposition::Retained)\n    }\n}\n\n/// A mapped controller observed disabled while PCI bus mastering was off.",
        replace: "    const fn must_retain(&self) -> bool {\n        matches!(&self.disposition, DmaDisposition::Retained)\n    }\n}\n\nfn alias_dma_state(controller: &Controller) {\n    let _ = &controller.dma;\n}\n\n/// A mapped controller observed disabled while PCI bus mastering was off.",
        breaks: "DMA state gains an unaudited whole-state access alias",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::dma_disposition_cannot_move_between_controllers",
        },
    },
    Mutation {
        id: 993,
        file: "kernel/src/fs.rs",
        find: "pub(crate) const VOLUME_SECTOR: u64 = 2048;",
        replace: "pub(crate) const VOLUME_SECTOR: u64 = 2048;\n\nfn with_volume() {}",
        breaks: "filesystem volume ownership regrows a callback authority door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::address_space::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 994,
        file: "kernel/src/nvme.rs",
        find: "const COMMAND_TIMEOUT_MS: u64 = 5_000;",
        replace: "const COMMAND_TIMEOUT_MS: u64 = 5_000;\n\nfn with_controller() {}",
        breaks: "NVMe controller ownership regrows a callback authority door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::address_space::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 995,
        file: "kernel/src/tmpfs.rs",
        find: "pub(crate) const ROOT: usize = 0;",
        replace: "pub(crate) const ROOT: usize = 0;\n\nfn walk() {}",
        breaks: "tmpfs regrows a speculative internal pathname walker",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::address_space::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 1590,
        file: "kernel/src/nvme/preparation.rs",
        find: "fn finish_preparation(\n    registers: MappedBar,\n    capabilities: Capabilities,\n    admin: QueuePair,\n    capacity: Capacity,\n    #[cfg(feature = \"selftest\")] observed_window: u64,\n) -> PrepareOutcome {\n",
        replace: "#[inline(never)]\nfn finish_preparation(\n    registers: MappedBar,\n    capabilities: Capabilities,\n    admin: QueuePair,\n    capacity: Capacity,\n    #[cfg(feature = \"selftest\")] observed_window: u64,\n) -> PrepareOutcome {\n    #[cfg(debug_assertions)]\n    let stack_pressure = [0u8; 8 * 1024];\n    #[cfg(not(debug_assertions))]\n    let stack_pressure = [0u8; 16 * 1024];\n    core::hint::black_box(&stack_pressure);\n",
        breaks: "a frame below preparation's register accesses consumes the boot-stack margin unseen by the live-leaf meter",
        reddens: Caught::Check("nvme candidates preserve two pages of boot-stack headroom"),
    },
    Mutation {
        id: 1593,
        file: "kernel/src/nvme.rs",
        find: "fn read(registers: &MappedBar, offset: usize) -> Result<u32, BringUpError> {\n    #[cfg(feature = \"selftest\")]\n    observe_candidate_stack();",
        replace: "fn read(registers: &MappedBar, offset: usize) -> Result<u32, BringUpError> {\n    observe_candidate_stack();",
        breaks: "every shipped NVMe register read runs the stack meter's leaf hook",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine::machine_evidence_writes_are_absent_from_shipped_paths",
        },
    },
    Mutation {
        id: 1643,
        file: "kernel/src/nvme/queues.rs",
        find: "        if !completion_mapped {\n            if isolation.unmap_segment(&mut self.submission).is_ok() {\n                self.submission_mapped = false;\n            }\n            return false;\n        }",
        replace: "        if !completion_mapped {\n            return false;\n        }",
        breaks: "a completion-queue mapping refusal leaves the submission queue reachable",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::nvme::a_partial_queue_mapping_rolls_back_the_submission_queue",
        },
    },
    Mutation {
        id: 1653,
        file: "kernel/src/nvme/io.rs",
        find: "        if !self.unmap_queues() {\n            warn!(\"nvme: a queue mapping stayed reachable during retirement\");\n            return quarantine_controller(self);\n        }",
        replace: "",
        breaks: "clean controller retirement drops queue owners without invalidating their domain leaves",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::rejected_controller_retirement_fences_and_unmaps_every_queue",
        },
    },
    Mutation {
        id: 1654,
        file: "kernel/src/nvme/queues.rs",
        find: "        self.retain_mapped();",
        replace: "        let _still_mapped = self.has_mappings();",
        breaks: "an accidentally dropped mapped queue can recycle device-reachable frames",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::retirement::rejected_controller_retirement_fences_and_unmaps_every_queue",
        },
    },
    Mutation {
        id: 1657,
        file: "kernel/src/nvme/io.rs",
        find: "        self.io = Some(pair);",
        replace: "        let _orphaned_queue_owner = pair;",
        breaks: "an uncertain partial I/O queue map falls out of the controller retirement inventory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::nvme::uncertain_io_queue_mapping_stays_reachable_for_retirement",
        },
    },
    Mutation {
        id: 1660,
        file: "kernel/src/nvme/queues.rs",
        find: "            Ok(()) => self.submission_mapped = true,",
        replace: "            Ok(()) => {}",
        breaks: "a successfully mapped submission queue is absent from conservative teardown state",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::nvme::queue_mapping_state_conserves_every_reachable_segment",
        },
    },
    Mutation {
        id: 1661,
        file: "kernel/src/nvme/queues.rs",
        find: "        if self.submission_mapped {\n            self.submission.retain();\n        }",
        replace: "        let _submission_may_be_mapped = self.submission_mapped;",
        breaks: "the mapped-queue drop backstop omits a conservatively reachable submission segment",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::nvme::queue_mapping_state_conserves_every_reachable_segment",
        },
    },
    Mutation {
        id: 1662,
        file: "kernel/src/nvme/queues.rs",
        find: "        if self.submission_mapped && isolation.unmap_segment(&mut self.submission).is_ok() {\n            self.submission_mapped = false;\n        }",
        replace: "        if self.submission_mapped && isolation.unmap_segment(&mut self.submission).is_ok() {\n            self.submission_mapped = true;\n        }",
        breaks: "a successfully invalidated submission queue stays marked live and forces false quarantine",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::nvme::queue_mapping_state_conserves_every_reachable_segment",
        },
    },
    Mutation {
        id: 1792,
        file: "kernel/src/nvme/io.rs",
        find: "            && isolation.map_segment_for_device_write(buffer).is_err()",
        replace: "            && isolation.map_segment(buffer).is_err()",
        breaks: "an NVMe read buffer gives the device read access to host residue",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::vtd::vtd_publication_and_nvme_buffers_keep_exact_dma_authority",
        },
    },
    Mutation {
        id: 1798,
        file: "kernel/src/nvme/queues.rs",
        find: "map_segment_for_device_read(&mut self.submission)",
        replace: "map_segment(&mut self.submission)",
        breaks: "the NVMe submission queue grants the device write access",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::vtd::vtd_publication_and_nvme_buffers_keep_exact_dma_authority",
        },
    },
    Mutation {
        id: 1799,
        file: "kernel/src/nvme/queues.rs",
        find: "map_segment_for_device_write(&mut self.completion)",
        replace: "map_segment(&mut self.completion)",
        breaks: "the NVMe completion queue grants the device read access",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::vtd::vtd_publication_and_nvme_buffers_keep_exact_dma_authority",
        },
    },
];
