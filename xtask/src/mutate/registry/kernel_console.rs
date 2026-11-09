use super::{Caught, Mutation};

pub(super) const MUTATIONS: &[Mutation] = &[
    Mutation {
        id: 57,
        file: "kernel/src/input.rs",
        find: "            KeyState::Released => {\n                if self.down & bit != 0 {\n                    self.completed[index] = self.completed[index].wrapping_add(1);\n                }\n                self.down &= !bit;\n            }",
        replace: "            KeyState::Released => {\n                self.completed[index] = self.completed[index].wrapping_add(1);\n                self.down &= !bit;\n            }",
        breaks: "an interrupt witness accepts a release with no preceding press",
        reddens: Caught::Check("the controller route and ordered interrupt witness are exact"),
    },
    // Without the fixup the fault is terminal, so its panic is the causal observer.
    Mutation {
        id: 174,
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "            if usercopy::recover(frame, fault) =>",
        replace: "            if false && usercopy::recover(frame, fault) =>",
        breaks: "a faulting user copy resumes instead of stopping the machine",
        reddens: Caught::Panic {
            marker: "faulting address 0x0000600000000000",
            why: "without the fixup the fault is terminal, so the kernel dies before it \
                  reaches a verdict at all",
        },
    },
    // Retained Phase 0-5 audit checks (D-135 to D-145).
    Mutation {
        id: 188,
        file: "kernel/src/console.rs",
        find: "        self.hide_cursor();
        self.put(text);
        self.newline();
        self.show_cursor();",
        replace: "        self.hide_cursor();
        self.put(text);
        self.newline();",
        breaks: "a line break leaves its new cursor hidden",
        reddens: Caught::Check("a scroll moves one visible cursor"),
    },
    Mutation {
        id: 189,
        file: "kernel/src/dev.rs",
        find: "pub(crate) fn with_console(f: impl FnOnce(&mut Console)) {
    let mut guard = CONSOLE.lock();
    if let Some(console) = guard.as_mut() {
        f(console);
    }
}",
        replace: "pub(crate) fn with_console(f: impl FnOnce(&mut Console)) {
    let disabled = wost::arch::current::interrupt::disable();
    let mut guard = CONSOLE.lock();
    if let Some(console) = guard.as_mut() {
        f(console);
    }
    disabled.restore();
}",
        breaks: "console work keeps interrupts available",
        reddens: Caught::Check("a console scroll does not block interrupts"),
    },
    // QEMU makes the bit and identity agree, so only omitted re-derivation is observable.
    Mutation {
        id: 289,
        file: "kernel/src/input.rs",
        find: "    keyboard.decoder = decoder_for(&mut keyboard.controller);",
        replace: "    let _ = &mut keyboard.controller;",
        breaks: "translation leaves the decoder on its previous numbering",
        reddens: Caught::Check("translation updates the decoder from the controller identity"),
    },
    // Seeding against the stream makes an unadopted numbering change observable.
    Mutation {
        id: 290,
        file: "kernel/src/input.rs",
        find: "        if translated != self.decoder.reads_translated() {",
        replace: "        if false && translated != self.decoder.reads_translated() {",
        breaks: "the numbering the stream proved is the one the decoder adopts",
        reddens: Caught::Check("the keyboard's numbering and the decoder's are the same"),
    },
    Mutation {
        id: 294,
        file: "kernel/src/input.rs",
        find: "pub(crate) const HUMAN_WAIT_MS: u64 = 45_000;",
        replace: "pub(crate) const HUMAN_WAIT_MS: u64 = 90_000;",
        breaks: "three unanswered keystroke rounds end as a red check, not a hang",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::kernel_source::the_human_wait_fits_the_harness_patience",
        },
    },
    Mutation {
        id: 407,
        file: "kernel/src/panic.rs",
        find: "        let _ = write_record(uart, thread, info);",
        replace: "        let _ = writeln!(uart, \"[error] KERNEL PANIC on thread '{thread}': {info}\");",
        breaks: "the kernel writes PanicInfo bytes around the single-line adapter",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::kernel_source::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        id: 412,
        file: "kernel/src/arch/x86_64/uart.rs",
        find: "        if !self.wait_for(line_status::TRANSMIT_HOLDING_EMPTY) {\n            return Err(fmt::Error);\n        }",
        replace: "        let _ = self.wait_for(line_status::TRANSMIT_HOLDING_EMPTY);",
        breaks: "the UART reports a refused byte as transmitted",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::kernel_source::the_uart_reports_every_refused_byte_to_its_formatter",
        },
    },
    Mutation {
        id: 413,
        file: "kernel/src/arch/x86_64/uart.rs",
        find: "                self.write_byte(b'\\r')?;",
        replace: "                let _ = self.write_byte(b'\\r');",
        breaks: "the UART formatter hides a refused carriage return",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::kernel_source::the_uart_reports_every_refused_byte_to_its_formatter",
        },
    },
    Mutation {
        id: 414,
        file: "kernel/src/arch/x86_64/uart.rs",
        find: "            self.write_byte(byte)?;",
        replace: "            let _ = self.write_byte(byte);",
        breaks: "the UART formatter hides a refused payload byte",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::kernel_source::the_uart_reports_every_refused_byte_to_its_formatter",
        },
    },
    Mutation {
        id: 561,
        file: "kernel/src/arch/x86_64/i8042.rs",
        find: "    fn require_config(&mut self, value: u8) -> Result<(), I8042Error> {\n        self.write_config(value)\n            .then_some(())\n            .ok_or(I8042Error::ConfigurationRefused)\n    }",
        replace: "    fn require_config(&mut self, value: u8) -> Result<(), I8042Error> {\n        let _ = self.write_config(value);\n        Ok(())\n    }",
        breaks: "a required controller configuration refusal is silently accepted",
        reddens: Caught::Check("a refused controller configuration is typed and changes nothing"),
    },
    Mutation {
        id: 562,
        file: "kernel/src/arch/x86_64/i8042.rs",
        find: "    pub(crate) fn enable_interrupt(&mut self) -> Result<(), InterruptEnableError> {\n        if !Self::flush(self) {\n            return Err(InterruptEnableError::Unavailable);\n        }",
        replace: "    pub(crate) fn enable_interrupt(&mut self) -> Result<(), InterruptEnableError> {\n        let _ = Self::flush(self);",
        breaks: "an i8042 transaction proceeds through stale undrained output",
        reddens: Caught::Check("a failed controller flush stops every following transaction"),
    },
    Mutation {
        id: 563,
        file: "kernel/src/arch/x86_64/i8042.rs",
        find: "    fn controller_query(&mut self, command: u8) -> Option<u8> {\n        if !self.controller_command(command) {\n            return None;\n        }\n        self.read_reply(RESPONSE_ATTEMPTS)\n    }",
        replace: "    fn controller_query(&mut self, command: u8) -> Option<u8> {\n        let _ = self.controller_command(command);\n        self.read_reply(RESPONSE_ATTEMPTS)\n    }",
        breaks: "a refused i8042 command consumes an unrelated reply byte",
        reddens: Caught::Check("a refused controller command is typed and consumes no reply"),
    },
    Mutation {
        id: 605,
        file: "kernel/src/input.rs",
        find: "fn record_interrupt_handoff(latch: &Once) {\n    assert!(\n        latch.claim(),\n        \"the keyboard interrupt handoff happened more than once\"\n    );\n}",
        replace: "fn record_interrupt_handoff(latch: &Once) {\n    let _ = latch;\n}",
        breaks: "the routed keyboard never records its interrupt-driven handoff",
        reddens: Caught::Check("the keyboard interrupt handoff records its ownership"),
    },
    Mutation {
        id: 772,
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "                if let Some(eoi) = eoi {\n                    dismiss(eoi);\n                }",
        replace: "                if let Some(eoi) = eoi {\n                    drop(eoi);\n                }",
        breaks: "the ping interrupt arm drops its completion authority without issuing EOI",
        reddens: Caught::Check("the interrupt is dismissed, so the next one arrives"),
    },
    Mutation {
        id: 773,
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "    drop(eoi);\n    let reclaimed = apic::claim_eoi(frame, vector::PING);",
        replace: "    core::mem::forget(eoi);\n    let reclaimed = apic::claim_eoi(frame, vector::PING);",
        breaks: "abandoning an EOI token can permanently strand its live-claim latch",
        reddens: Caught::Panic {
            marker: "interrupts became enabled inside an exclusion",
            why: "forgetting the EOI token strands its interrupt exclusion, so trap return \
                  re-enables interrupts inside that exclusion before the next claim",
        },
    },
    Mutation {
        id: 775,
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "            dismiss(eoi);\n            crate::thread::reschedule(frame);",
        replace: "            drop(eoi);\n            crate::thread::reschedule(frame);",
        breaks: "the timer interrupt arm drops its completion authority without issuing EOI",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic::every_interrupt_arm_has_explicit_completion_ownership",
        },
    },
    Mutation {
        id: 776,
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "            let woke = crate::input::on_interrupt();\n            dismiss(eoi);",
        replace: "            let woke = crate::input::on_interrupt();\n            drop(eoi);",
        breaks: "the keyboard interrupt arm drops its completion authority without issuing EOI",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic::every_interrupt_arm_has_explicit_completion_ownership",
        },
    },
    Mutation {
        id: 836,
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "        write(&mut self.registers, low, entry::MASKED)?;\n        let (observed_low, observed_high) = self.entry_words(line)?;\n        if observed_low != entry::MASKED {\n            return Err(MaskError::Refused {\n                low: observed_low,\n                high: observed_high,\n            });\n        }\n        write(&mut self.registers, low + 1, 0)?;",
        replace: "        write(&mut self.registers, low, entry::MASKED)?;\n        write(&mut self.registers, low + 1, 0)?;",
        breaks: "an unproven takeover mask permits a destination write",
        reddens: Caught::Check(
            "a refused keyboard route restores configuration, masking and mappings",
        ),
    },
    Mutation {
        id: 837,
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "        if low != entry::MASKED || high != 0 {\n            return Err(MaskError::Refused { low, high });\n        }",
        replace: "        let _ = (low, high);",
        breaks: "an ignored takeover destination clear is accepted",
        reddens: Caught::Check(
            "a refused keyboard route restores configuration, masking and mappings",
        ),
    },
    Mutation {
        id: 838,
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "            Ok((low, high)) if low == expected_low && high == destination => Ok(()),\n            Ok((low, high)) => self.refuse_route(line, expected_low, destination, low, high),",
        replace: "            Ok((_low, _high)) => Ok(()),",
        breaks: "an ignored unmask write is accepted",
        reddens: Caught::Check(
            "a refused keyboard route restores configuration, masking and mappings",
        ),
    },
    Mutation {
        id: 839,
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "        match self.entry_words(line) {\n            Ok((low, high)) if low == entry::MASKED && high == destination => {}\n            Ok((low, high)) => {\n                return self.refuse_route(line, expected_low, destination, low, high);\n            }\n            Err(error) => {\n                self.mask_line_and_verify(line)\n                    .map_err(RoutingError::from)?;\n                return Err(RoutingError::Registers(error));\n            }\n        }",
        replace: "        let _ = self.entry_words(line);",
        breaks: "an IOAPIC destination is unmasked before its readback",
        reddens: Caught::Check(
            "a refused keyboard route restores configuration, masking and mappings",
        ),
    },
    Mutation {
        id: 932,
        file: "kernel/src/input.rs",
        find: "pub(crate) fn set_translation(translation: crate::arch::current::i8042::Translation) -> bool {",
        replace: "pub(crate) fn set_translation(translation: bool) -> bool {",
        breaks: "input translation policy becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 933,
        file: "kernel/src/arch/x86_64/i8042.rs",
        find: "    pub(crate) fn set_translation(&mut self, translation: Translation) -> bool {",
        replace: "    pub(crate) fn set_translation(&mut self, translation: bool) -> bool {",
        breaks: "i8042 translation policy becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 936,
        file: "kernel/src/console.rs",
        find: "    fn show_cursor(&mut self) {",
        replace: "    fn show_cursor(&mut self, _visible: bool) {",
        breaks: "cursor display regains a redundant unnamed visibility boolean",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 937,
        file: "kernel/src/console.rs",
        find: "    fn hide_cursor(&mut self) {",
        replace: "    fn hide_cursor(&mut self, _visible: bool) {",
        breaks: "cursor hiding regains a redundant unnamed visibility boolean",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 938,
        file: "kernel/src/console.rs",
        find: "    fn show_cursor(&mut self) {",
        replace: "    fn set_cursor(&mut self, visible: bool) {\n        let colour = if visible {\n            self.palette.foreground\n        } else {\n            self.palette.background\n        };\n        self.paint_cursor(colour);\n    }\n\n    fn show_cursor(&mut self) {",
        breaks: "cursor visibility regrows one boolean-shaped operation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 1271,
        file: "kernel/src/arch/x86_64/vector.rs",
        find: "pub(crate) const SYSCALL: InterruptVector = SOURCES[4];",
        replace: "pub(crate) const SYSCALL: InterruptVector = SOURCES[3];",
        breaks: "the syscall gate aliases the local APIC timer's assigned vector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::abi::the_c_header_and_the_kernel_agree_about_the_abi",
        },
    },
    Mutation {
        id: 1272,
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "    fn route_line(\n",
        replace: "    #[allow(dead_code)]\n    fn route_raw_for_mutation(\n        &mut self,\n        line: u32,\n        vector: u8,\n        wiring: Wiring,\n        destination: u32,\n    ) -> Result<(), RoutingError> {\n        let Some(vector) = InterruptVector::new(vector) else {\n            return Ok(());\n        };\n        self.route_line(line, vector, wiring, destination)\n    }\n\n    fn route_line(\n",
        breaks: "an I/O APIC programming door recovers a raw vector parameter",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic::one_assignment_and_typed_signatures_own_every_vector_door",
        },
    },
    Mutation {
        id: 1273,
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "        && controller.registers.low == u32::from(super::vector::KEYBOARD.get())",
        replace: "        && controller.registers.low == u32::from(super::vector::TIMER.get())",
        breaks: "the isolated I/O APIC route accepts another source's encoded vector",
        reddens: Caught::Check("an isolated I/O APIC route encodes its typed vector"),
    },
    Mutation {
        id: 1346,
        file: "kernel/src/dev.rs",
        find: "pub(crate) fn keep_io_apic(io_apic: IoApic, #[cfg(feature = \"selftest\")] gsi: u32) {",
        replace: "pub(crate) fn keep_io_apic(io_apic: IoApic, gsi: u32) {",
        breaks: "the routed-GSI evidence parameter reaches the shipped helper",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine::machine_evidence_writes_are_absent_from_shipped_paths",
        },
    },
    Mutation {
        id: 1349,
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "            #[cfg(not(feature = \"selftest\"))]\n            dismiss(eoi);",
        replace: "            #[cfg(not(feature = \"selftest\"))]\n            {\n                let eoi = Some(eoi);\n                if let Some(eoi) = eoi {\n                    dismiss(eoi);\n                }\n            }",
        breaks: "the shipped ping path wraps completion authority for test-only recovery",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic::every_interrupt_arm_has_explicit_completion_ownership",
        },
    },
    Mutation {
        id: 1532,
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "        let Some(entries) = entries_from_version(version) else {\n            return Err(TakeFailure {\n                registers,\n                reason: TakeError::ImplausibleVersion(version),\n            });\n        };",
        replace: "        let entries = ((version >> 16) & 0xFF) + 1;",
        breaks: "a window firmware aimed elsewhere is taken over as an I/O APIC",
        reddens: Caught::Check("a window that is not an I/O APIC is never taken over"),
    },
    Mutation {
        id: 1533,
        file: "kernel/src/arch/x86_64/debug_exit.rs",
        find: "    if let Some(mut device) = port::debug_exit() {",
        replace: "    if let Some(mut device) = port::claim::<u32>(0xF4) {",
        breaks: "a production driver claims a port by its number",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::ports::production_kernel_code_claims_no_port_by_number",
        },
    },
    Mutation {
        id: 1581,
        file: "kernel/src/arch/x86_64/uart.rs",
        find: "        let registers = port::com1().ok_or(UartError::PortsUnavailable)?;",
        replace: "        let registers = { use wost::arch::current::port::claim_block; claim_block::<8>(0x3F8) }.ok_or(UartError::PortsUnavailable)?;",
        breaks: "a production driver claims a port through an imported raw door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::ports::production_kernel_code_claims_no_port_by_number",
        },
    },
    Mutation {
        id: 1582,
        file: "kernel/src/arch/x86_64/i8042.rs",
        find: "        let (data, control) = port::i8042().ok_or(I8042Error::PortsUnavailable)?;",
        replace: "        let (data, control) = { use wost::arch::current::port::claim_pair as pair; pair::<u8, u8>(0x60, 0x64) }.ok_or(I8042Error::PortsUnavailable)?;",
        breaks: "a production driver claims a port through a renamed raw door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::ports::production_kernel_code_claims_no_port_by_number",
        },
    },
    Mutation {
        id: 1583,
        file: "kernel/src/arch/x86_64/pit.rs",
        find: "        let registers = port::pit().ok_or(PitError::PortsUnavailable)?;",
        replace: "        let registers = { use wost::arch::current::port as io; io::claim_block::<4>(0x40) }.ok_or(PitError::PortsUnavailable)?;",
        breaks: "a production driver claims a port through a renamed port module",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::ports::production_kernel_code_claims_no_port_by_number",
        },
    },
    Mutation {
        id: 1584,
        file: "kernel/src/arch/x86_64/debug_exit.rs",
        find: "    if let Some(mut device) = port::debug_exit() {",
        replace: "    if let Some(mut device) = { use wost::arch::current::port::*; claim::<u32>(0xF4) } {",
        breaks: "a production driver claims a port through a glob import of the port module",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::ports::production_kernel_code_claims_no_port_by_number",
        },
    },
    Mutation {
        id: 1725,
        file: "kernel/src/input.rs",
        find: "        if crate::time::is_calibrated() && interrupt::are_enabled() {\n            crate::thread::sleep_ms(&mut context, POLL_INTERVAL_MS);",
        replace: "        if crate::time::is_calibrated() {\n            crate::thread::sleep_ms(&mut context, POLL_INTERVAL_MS);",
        breaks: "the keyboard poller sleeps forever when timer state exists but interrupts stay off",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::entry::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 2435,
        file: "kernel/src/input.rs",
        find: "pub(crate) fn try_read_key_for_test() -> Option<Keystroke> {\n    EVENTS.lock_leaf().pop()\n}",
        replace: "pub(crate) fn try_read_key_for_test() -> Option<Keystroke> {\n    None\n}",
        breaks: "the nonblocking self-test reader refuses even an available queued event",
        reddens: Caught::Check("a delayed read preserves interrupt admission and exact teardown"),
    },
    Mutation {
        id: 2436,
        file: "kernel/src/input.rs",
        find: "    let exclusion = interrupt::disable();\n    let mut keystrokes = [None; BATCH];\n    let count = drain(&mut keystrokes);\n    after_drain();",
        replace: "    let mut keystrokes = [None; BATCH];\n    let count = drain(&mut keystrokes);\n    after_drain();\n    let exclusion = interrupt::disable();",
        breaks: "Acquiring exclusion only after the drain-to-delivery boundary lets a competing producer admit its later batch before the first batch.",
        reddens: Caught::Check("the controller route and ordered interrupt witness are exact"),
    },
    Mutation {
        id: 2437,
        file: "kernel/src/input.rs",
        find: "    if admitted {\n        observe(keystroke.event);\n    }",
        replace: "    observe(keystroke.event);",
        breaks: "A rejected full-queue press/release pair advances the self-test arrival watch even though no event was admitted.",
        reddens: Caught::Check("the controller route and ordered interrupt witness are exact"),
    },
    Mutation {
        id: 2463,
        file: "kernel/src/arch/x86_64/i8042.rs",
        find: "        discard();
    }
    !full()
}",
        replace: "        discard();
    }
    false
}",
        breaks: "a controller that drains on its last allowed read is reported absent",
        reddens: Caught::Check("an 8042 backlog of exactly the flush bound drains"),
    },
];
