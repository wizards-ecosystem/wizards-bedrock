use super::{Caught, Mutation};

pub(super) const MUTATIONS: &[Mutation] = &[
    // Two offsets in one header treated as two device identities. Exact-offset
    // duplicate checks cannot catch this; the claim must canonicalise first.
    Mutation {
        id: 213,
        file: "libs/pci-config/src/lib.rs",
        find: "    let at = at.function_base();
        if self.is_claimed(at) {",
        replace: "    let at = at;
        if self.is_claimed(at) {",
        breaks: "configuration offsets alias one function claim",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::function_claims_validate_headers_and_aliases_before_commit",
        },
    },
    // A bridge's bus numbers decoded as a base address and then probed —
    // all ones written into the register that says which bus is behind it.
    Mutation {
        id: 216,
        file: "libs/pci-config/src/lib.rs",
        find: "    match probe_bars_unleased(authority.bar_registers(), |event| access(function, event)) {",
        replace: "    match probe_bars_unleased(BARS, |event| access(function, event)) {",
        breaks: "a bridge has two base address registers, not six",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::the_claimed_header_alone_sets_the_public_bar_probe_extent",
        },
    },
    // Decode left enabled while BARs hold all ones; a fake device observes
    // the cycle where QEMU might forgive it.
    Mutation {
        id: 217,
        file: "libs/pci-config/src/lib.rs",
        find: "const PROBE_DECODE: u16 = command::IO_SPACE | command::MEMORY_SPACE;",
        replace: "const PROBE_DECODE: u16 = 0;",
        breaks: "bar probing disables both decode spaces",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bar_probe_is_one_restored_decode_off_transaction",
        },
    },
    // A window one past the port-space width must not truncate to zero.
    Mutation {
        id: 218,
        file: "libs/pci-config/src/lib.rs",
        find: "    let mask = register_bits::u16_halves(mask).0 & !0x3;\n    if mask == 0 {",
        replace: "    let mask = register_bits::u16_halves(mask).0 & !0x3;\n    if false {",
        breaks: "a probe that decodes no port bits sizes a 65,536-port window truncated to zero ports",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_probe_that_decoded_nothing_is_refused",
        },
    },
    // An out-of-range register count reaches past the fixed transaction arrays,
    // while every smaller header is refused before its first access.
    Mutation {
        id: 219,
        file: "libs/pci-config/src/lib.rs",
        find: "    if registers > BARS {",
        replace: "    if registers < BARS {",
        breaks: "bar probing accepts exactly the bounded header register counts",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bar_probe_refuses_only_register_counts_above_its_bound",
        },
    },
    // The high half is no longer all ones when the low mask is read.
    Mutation {
        id: 220,
        file: "libs/pci-config/src/lib.rs",
        find: "        if let Some(high) = slots.high {
            modified |= 1 << high;",
        replace: "        if false && let Some(high) = slots.high {
            modified |= 1 << high;",
        breaks: "both halves of a wide bar are probed together",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bar_probe_is_one_restored_decode_off_transaction",
        },
    },
    // Bit two is an address bit in an I/O BAR, not the wide-memory type.
    // Treating it as width consumes the next header field.
    Mutation {
        id: 221,
        file: "libs/pci-config/src/lib.rs",
        find: "        let high = if matches!(
            Bar::decode(original[low], 0)?,
            Bar::Memory { wide: true, .. }
        ) {",
        replace: "        let high = if original[low] & 0b110 == 0b100 {",
        breaks: "an io address bit cannot consume a second bar slot",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::an_io_address_bit_is_not_a_wide_memory_type",
        },
    },
    // A zero base is still an implemented narrow memory BAR when its probe
    // mask reports a window.
    Mutation {
        id: 222,
        file: "libs/pci-config/src/lib.rs",
        find: "                let Some(size) = memory_window_size(masks[index], 0, MemoryWidth::Bits32) else {",
        replace: "                let Some(size) = memory_window_size(0, 0, MemoryWidth::Bits32) else {",
        breaks: "an address-zero memory bar remains discoverable",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::an_address_zero_memory_bar_is_discovered_from_its_mask",
        },
    },
    // A successful probe returns while the device still holds probe values.
    Mutation {
        id: 223,
        file: "libs/pci-config/src/lib.rs",
        find: "    if restore.phase == ProbeRestorePhase::Bars {",
        replace: "    if false {",
        breaks: "a successful BAR probe restores every BAR before staging its command",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bar_probe_is_one_restored_decode_off_transaction",
        },
    },
    // Rollback restores BARs but forgets the command register after an access
    // fails; exhaustive cycle injection reaches every such exit.
    Mutation {
        id: 224,
        file: "libs/pci-config/src/lib.rs",
        find: "fn after_probe_error<E>(
    reason: ProbeError<E>,
    access: &mut impl FnMut(ProbeAccess) -> Result<u32, E>,
    restore: ProbeRestore,
) -> RawProbeFailure<E> {
    match finish_probe_restoration(restore, access) {",
        replace: "fn after_probe_error<E>(
    reason: ProbeError<E>,
    access: &mut impl FnMut(ProbeAccess) -> Result<u32, E>,
    mut restore: ProbeRestore,
) -> RawProbeFailure<E> {
    restore.command &= !PROBE_DISABLED;
    match finish_probe_restoration(restore, access) {",
        breaks: "a failed BAR probe restores decode and DMA state",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_probe_access_failure_restores_the_device",
        },
    },
    // An address keeps its register offset when asked for its device identity.
    Mutation {
        id: 227,
        file: "libs/pci-config/src/lib.rs",
        find: "        Self(self.0 & !0xFF)",
        replace: "        Self(self.0)",
        breaks: "a function identity contains no register offset",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_offset_of_one_function_has_one_identity",
        },
    },
    // Consuming the probe result must retain every decoded window.
    Mutation {
        id: 228,
        file: "libs/pci-config/src/lib.rs",
        find: "    pub const fn into_windows(self) -> [Option<Window>; BARS] {
        self.windows
    }",
        replace: "    pub const fn into_windows(self) -> [Option<Window>; BARS] {
        [None; BARS]
    }",
        breaks: "consuming probed bars returns their decoded windows",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::probed_bars_return_their_complete_slot_array",
        },
    },
    // Report release while retaining the command sublease.
    Mutation {
        id: 229,
        file: "libs/pci-config/src/lib.rs",
        find: "        *slot = None;
        true",
        replace: "        *slot = Some(lease);
        true",
        breaks: "a settled PCI command returns its exact registry slot",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::command_leases_refuse_nesting_and_preserve_uncertain_retry_authority",
        },
    },
    // A wide BAR in a bridge's last slot would consume its bus-number field.
    Mutation {
        id: 230,
        file: "libs/pci-config/src/lib.rs",
        find: "            Some(core::iter::Iterator::next(&mut indices)?)",
        replace: "            let _ = core::iter::Iterator::next(&mut indices);
            None",
        breaks: "a wide bar cannot consume the field after the header's last slot",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_wide_bar_in_the_final_slot_is_refused_before_mutation",
        },
    },
    // A wide non-prefetchable BAR must not acquire a property merely because
    // other bits in the low word are set.
    Mutation {
        id: 231,
        file: "libs/pci-config/src/lib.rs",
        find: "                address: base | ((high as u64) << 32),
                wide: true,
                prefetchable: low & Self::PREFETCHABLE != 0,",
        replace: "                address: base | ((high as u64) << 32),
                wide: true,
                prefetchable: true,",
        breaks: "a wide bar reports only its actual prefetchable bit",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_base_address_register_decodes_to_its_window",
        },
    },
    // A non-power-of-two mask happens to be aligned and otherwise plausible.
    Mutation {
        id: 232,
        file: "libs/pci-config/src/lib.rs",
        find: "        if !size.is_power_of_two() || !base.is_multiple_of(size) {",
        replace: "        if false || !base.is_multiple_of(size) {",
        breaks: "a bar window size is a power of two",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::an_invalid_bar_mask_is_refused_after_restoration",
        },
    },
    // A power-of-two mask does not authorize a base in the middle of that
    // window's own alignment unit.
    Mutation {
        id: 233,
        file: "libs/pci-config/src/lib.rs",
        find: "        if !size.is_power_of_two() || !base.is_multiple_of(size) {",
        replace: "        if !size.is_power_of_two() || false {",
        breaks: "a bar base is aligned to its decoded window size",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_misaligned_bar_base_is_refused_after_restoration",
        },
    },
    // The first refused restore aborts rollback instead of continuing through
    // the command register and every older BAR.
    Mutation {
        id: 234,
        file: "libs/pci-config/src/lib.rs",
        find: "            if restore.modified(index) {
                let _ = access(ProbeAccess::WriteBar(index, restore.original[index]));
            }",
        replace: "            if restore.modified(index)
                && let Err(error) = access(ProbeAccess::WriteBar(index, restore.original[index]))
            {
                return Err((ProbeError::Restore(error), restore));
            }",
        breaks: "bar rollback continues after a restore is refused",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_restore_failure_still_attempts_every_later_restore",
        },
    },
    // Require memory decode but omit bus master from the transaction. MMIO
    // works and every DMA queue then waits forever without an error register.
    Mutation {
        id: 236,
        file: "libs/pci-config/src/lib.rs",
        find: "        command::MEMORY_SPACE | command::BUS_MASTER,\n        0,\n        access,",
        replace: "        command::MEMORY_SPACE,\n        0,\n        access,",
        breaks: "bus master enable preserves and verifies every required command bit",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::bus_master_enable_is_verified_and_preserves_the_command",
        },
    },
    // If either required bit was already present, toggling instead of adding
    // both would turn the working half back off.
    Mutation {
        id: 237,
        file: "libs/pci-config/src/lib.rs",
        find: "    let installed = (before | set) & !clear;",
        replace: "    let installed = (before ^ set) & !clear;",
        breaks: "bus master enable preserves either required bit already set",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::bus_master_enable_preserves_either_already_required_bit",
        },
    },
    // Configuration failures are surfaced in the boot log, so an empty error
    // loses the only diagnosis available before a controller exists.
    Mutation {
        id: 238,
        file: "libs/pci-config/src/lib.rs",
        find: "            Self::Access => core::fmt::Write::write_str(f, \"configuration access failed\"),",
        replace: "            Self::Access => Ok(()),",
        breaks: "every pci enable error has a diagnostic description",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_enable_error_describes_its_cause",
        },
    },
    // Treat a successful configuration cycle as proof the device accepted the
    // command bits, skipping the readback which distinguishes the two claims.
    Mutation {
        id: 239,
        file: "libs/pci-config/src/lib.rs",
        find: "            Some(bits) if bits as u16 == wanted => return Ok(()),",
        replace: "            Some(_) => return Ok(()),",
        breaks: "a bus master command write is read back before success",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_refused_bus_master_enable_restores_the_exact_command",
        },
    },
    Mutation {
        id: 240,
        file: "libs/pci-config/src/lib.rs",
        find: "            Some(bits) if bits as u16 == wanted => return Ok(()),",
        replace: "            Some(bits) if bits as u16 & wanted == wanted => return Ok(()),",
        breaks: "an unwritten command bit can enable an unclaimed decoder",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bus_master_enable_refuses_and_restores_an_unwritten_command_bit",
        },
    },
    // Read the captured command word back without first restoring it.
    Mutation {
        id: 241,
        file: "libs/pci-config/src/lib.rs",
        find: "    let _ = access(at, CommandAccess::Write(state.before));\n    match access(at, CommandAccess::Read) {",
        replace: "    match access(at, CommandAccess::Read) {",
        breaks: "a successful pci lease restores its captured command word",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bus_master_lease_restores_the_exact_command_or_reports_uncertainty",
        },
    },
    // Return the initiating access error without restoring a command write
    // which may already have enabled only part of the device.
    Mutation {
        id: 242,
        file: "libs/pci-config/src/lib.rs",
        find: "    let _ = access(at, CommandAccess::Write(original));\n    match access(at, CommandAccess::Read) {",
        replace: "    return Err(failure);\n    #[expect(unreachable_code)]\n    match access(at, CommandAccess::Read) {",
        breaks: "every failed bus master access restores or reports uncertainty",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_failed_bus_master_access_restores_or_reports_uncertainty",
        },
    },
    Mutation {
        id: 749,
        file: "libs/pci-config/src/lib.rs",
        find: "    fn holds_function(&self, function: FunctionLease, lease: CommandLease) -> bool {\n        lease.function == function && self.slots.iter().flatten().any(|held| *held == lease)\n    }",
        replace: "    fn holds_function(&self, function: FunctionLease, lease: CommandLease) -> bool {\n        lease.function == function\n            && self.slots.iter().flatten().any(|held| {\n                held.function == lease.function && held.generation == lease.generation\n            })\n    }",
        breaks: "command state can be consumed through a different registry with the same function and generation",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::command_state_is_bound_to_its_registry_function_and_command_offset",
        },
    },
    Mutation {
        id: 750,
        file: "libs/pci-config/src/lib.rs",
        find: "    pub fn holds(&self, authority: &FunctionAuthority) -> bool {\n        self.slots\n            .iter()\n            .flatten()\n            .any(|held| *held == authority.lease)\n    }",
        replace: "    pub fn holds(&self, authority: &FunctionAuthority) -> bool {\n        let _ = authority;\n        true\n    }",
        breaks: "a foreign function authority reaches configuration cycles through another registry",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::configuration_cycles_require_the_issuing_function_registry",
        },
    },
    Mutation {
        id: 751,
        file: "libs/pci-config/src/lib.rs",
        find: "    if !functions.holds(authority) {\n        return Err(EnableFailure::from(EnableError::Stale));\n    }\n    enable_memory_space_with_leases(&mut functions.commands, authority, access)",
        replace: "    if functions.holds(authority) {\n        return Err(EnableFailure::from(EnableError::Stale));\n    }\n    enable_memory_space_with_leases(&mut functions.commands, authority, access)",
        breaks: "memory decode reverses which function registry may reach configuration space",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::public_memory_decode_accepts_only_its_function_authority",
        },
    },
    Mutation {
        id: 752,
        file: "libs/pci-config/src/lib.rs",
        find: "    admit_bus_master_with_leases(&mut functions.commands, authority.function, state, access)",
        replace: "    let _ = (&mut functions.commands, authority, state, access);\n    Ok(())",
        breaks: "bus-master admission reports success without changing the device or state",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::mapped_bus_master_admission_requires_its_bar_and_command_owner",
        },
    },
    Mutation {
        id: 753,
        file: "libs/pci-config/src/lib.rs",
        find: "    if !functions.holds_bar(authority) {\n        return Err(EnableError::Stale);\n    }\n    admit_bus_master_with_leases(&mut functions.commands, authority.function, state, access)",
        replace: "    if functions.holds_bar(authority) {\n        return Err(EnableError::Stale);\n    }\n    admit_bus_master_with_leases(&mut functions.commands, authority.function, state, access)",
        breaks: "bus-master admission rejects its live mapped BAR owner",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::mapped_bus_master_admission_requires_its_bar_and_command_owner",
        },
    },
    Mutation {
        id: 754,
        file: "libs/pci-config/src/lib.rs",
        find: "    if !functions.holds(authority) {\n        return Err(RestoreFailure {\n            reason: EnableError::Stale,\n            state,\n        });\n    }\n    commit_command_with_leases(&mut functions.commands, authority, state, access)",
        replace: "    if functions.holds(authority) {\n        return Err(RestoreFailure {\n            reason: EnableError::Stale,\n            state,\n        });\n    }\n    commit_command_with_leases(&mut functions.commands, authority, state, access)",
        breaks: "command commit rejects its live function authority before consuming the sublease",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::public_command_commit_consumes_the_live_sublease",
        },
    },
    Mutation {
        id: 755,
        file: "libs/pci-config/src/lib.rs",
        find: "    retry_probe_with_leases(&mut functions.commands, authority, retry, access)",
        replace: "    let _ = (&mut functions.commands, authority, retry, access);\n    Ok(())",
        breaks: "BAR retry reports success without restoring hardware or settling its sublease",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::public_probe_retry_restores_hardware_and_releases_its_sublease",
        },
    },
    Mutation {
        id: 756,
        file: "libs/pci-config/src/lib.rs",
        find: "    if !functions.holds(authority) {\n        return Err(ProbeFailure::with_retry(\n            ProbeError::Unavailable(EnableError::Stale),\n            retry,\n        ));\n    }\n    retry_probe_with_leases(&mut functions.commands, authority, retry, access)",
        replace: "    if functions.holds(authority) {\n        return Err(ProbeFailure::with_retry(\n            ProbeError::Unavailable(EnableError::Stale),\n            retry,\n        ));\n    }\n    retry_probe_with_leases(&mut functions.commands, authority, retry, access)",
        breaks: "BAR retry rejects the issuing registry before restoring its retained hardware state",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::public_probe_retry_restores_hardware_and_releases_its_sublease",
        },
    },
    Mutation {
        id: 757,
        file: "libs/pci-config/src/lib.rs",
        find: "    fn reserve(&mut self, authority: &FunctionAuthority) -> Result<CommandLease, EnableError> {
        if self
            .slots
            .iter()
            .flatten()
            .any(|lease| lease.function == authority.lease)
        {",
        replace: "    fn reserve(&mut self, authority: &FunctionAuthority) -> Result<CommandLease, EnableError> {
        if self
            .slots
            .iter()
            .flatten()
            .any(|lease| lease.function != authority.lease)
        {",
        breaks: "a second command transition can nest over an already leased function",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::command_leases_refuse_nesting_and_preserve_uncertain_retry_authority",
        },
    },
    Mutation {
        id: 758,
        file: "libs/pci-config/src/lib.rs",
        find: "    if current != state.installed {\n        return Err(RestoreFailure {\n            reason: EnableError::Changed(current),\n            state,\n        });\n    }\n    if leases.release(state.lease) {",
        replace: "    if leases.release(state.lease) {",
        breaks: "command commit can consume a lease after the installed word changed",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::command_consumption_requires_the_installed_word_and_live_generation",
        },
    },
    Mutation {
        id: 759,
        file: "libs/pci-config/src/lib.rs",
        find: "    fn holds_function(&self, function: FunctionLease, lease: CommandLease) -> bool {\n        lease.function == function && self.slots.iter().flatten().any(|held| *held == lease)\n    }",
        replace: "    fn holds_function(&self, _function: FunctionLease, lease: CommandLease) -> bool {\n        self.slots.iter().flatten().any(|held| {\n            held.registry == lease.registry && held.generation == lease.generation\n        })\n    }",
        breaks: "command state can be consumed for another function in the same registry generation",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::command_state_is_bound_to_its_registry_function_and_command_offset",
        },
    },
    Mutation {
        id: 760,
        file: "libs/pci-config/src/lib.rs",
        find: "    fn holds_function(&self, function: FunctionLease, lease: CommandLease) -> bool {\n        lease.function == function && self.slots.iter().flatten().any(|held| *held == lease)\n    }\n\n    fn release(&mut self, lease: CommandLease) -> bool {\n        let Some(slot) = self.slots.iter_mut().find(|held| **held == Some(lease)) else {",
        replace: "    fn holds_function(&self, function: FunctionLease, lease: CommandLease) -> bool {\n        lease.function == function\n            && self.slots.iter().flatten().any(|held| {\n                held.function == lease.function && held.generation == lease.generation\n            })\n    }\n\n    fn release(&mut self, lease: CommandLease) -> bool {\n        let Some(slot) = self.slots.iter_mut().find(|held| {\n            held.is_some_and(|held| {\n                held.function == lease.function && held.generation == lease.generation\n            })\n        }) else {",
        breaks: "a BAR probe token can release the matching generation from a different registry",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::bar_probe_reservations_are_registry_bound",
        },
    },
    Mutation {
        id: 761,
        file: "libs/pci-config/src/lib.rs",
        find: "            state: Some(state),",
        replace: "            state: None,",
        breaks: "an uncertain command transition loses the exclusive retry authority",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::command_leases_refuse_nesting_and_preserve_uncertain_retry_authority",
        },
    },
    Mutation {
        id: 762,
        file: "libs/pci-config/src/lib.rs",
        find: "    let wanted = current | command::MEMORY_SPACE | command::BUS_MASTER;\n    state.phase = CommandPhase::Restoring(wanted);\n    let result = change_command(at, current, wanted, &mut access);",
        replace: "    let wanted = current | command::MEMORY_SPACE | command::BUS_MASTER;\n    let result = change_command(at, current, wanted, &mut access);",
        breaks: "an interrupted command upgrade publishes no restoration phase before its write",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::an_interrupted_upgrade_records_retry_authority_before_the_write",
        },
    },
    Mutation {
        id: 763,
        file: "libs/pci-config/src/lib.rs",
        find: "        Address(self.function.at.0 | header::COMMAND as u32)",
        replace: "        Address(self.function.at.0 | header::BAR0 as u32)",
        breaks: "a command lease binds a BAR offset instead of the function command register",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::command_state_is_bound_to_its_registry_function_and_command_offset",
        },
    },
    Mutation {
        id: 764,
        file: "libs/pci-config/src/lib.rs",
        find: "        let Some(generation) = self.generation.checked_add(1) else {\n            return Err(EnableError::Exhausted);\n        };",
        replace: "        let generation = self.generation.wrapping_add(1);",
        breaks: "command generations wrap and replay instead of refusing exhaustion",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::command_generation_exhaustion_refuses_before_configuration_access",
        },
    },
    Mutation {
        id: 787,
        file: "libs/pci-config/src/lib.rs",
        find: "            Some(restore) => Err(ProbeFailure {\n                reason: failure.reason,\n                retry: Some(ProbeRetry { probe, restore }),\n            }),",
        replace: "            Some(restore) => Err(ProbeFailure {\n                reason: failure.reason,\n                retry: None,\n            }),",
        breaks: "an uncertain BAR restoration returns no exclusive retry authority",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::an_uncertain_bar_restore_retains_exclusion_and_retries_exactly",
        },
    },
    Mutation {
        id: 788,
        file: "libs/pci-config/src/lib.rs",
        find: "                Ok(bits) if bits == restore.original[index] => {}\n                Ok(_) => return Err((ProbeError::RestoreChanged, restore)),",
        replace: "                Ok(_) => {}",
        breaks: "BAR restoration accepts a write the device ignored",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::ignored_bar_and_command_restore_writes_retain_retry_authority",
        },
    },
    Mutation {
        id: 789,
        file: "libs/pci-config/src/lib.rs",
        find: "        Ok(bits) if bits as u16 == restore.command => Ok(()),\n        Ok(_) => Err((ProbeError::RestoreChanged, restore)),",
        replace: "        Ok(_) => Ok(()),",
        breaks: "command restoration accepts a write the device ignored",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::an_ignored_final_probe_command_restore_retains_retry_authority",
        },
    },
    Mutation {
        id: 790,
        file: "libs/pci-config/src/lib.rs",
        find: "            Ok(bits) if bits as u16 == decode_off => {}\n            Ok(_) => return Err((ProbeError::RestoreChanged, restore)),",
        replace: "            Ok(_) => {}",
        breaks: "a BAR retry restores windows without reproving that decode stayed off",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_retry_reproves_decode_is_off_before_restoring_bars",
        },
    },
    Mutation {
        id: 791,
        file: "libs/pci-config/src/lib.rs",
        find: "        Ok(bits) if bits as u16 == decode_off => {}\n        Ok(bits) => {\n            return Err(after_probe_error(\n                ProbeError::Refused(bits as u16),\n                &mut access,\n                command_restore,\n            ));\n        }",
        replace: "        Ok(_) => {}",
        breaks: "a BAR probe accesses windows after the device ignored decode-off",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::an_ignored_decode_off_write_is_refused_before_any_bar_access",
        },
    },
    Mutation {
        id: 792,
        file: "libs/pci-config/src/lib.rs",
        find: "            Some(restore) => Err(ProbeFailure {\n                reason: failure.reason,\n                retry: Some(ProbeRetry { probe, restore }),\n            }),",
        replace: "            Some(restore) => {\n                let _released = leases.release(probe.0);\n                Err(ProbeFailure {\n                    reason: failure.reason,\n                    retry: Some(ProbeRetry { probe, restore }),\n                })\n            }",
        breaks: "an uncertain BAR retry token releases its exclusion lease before return",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::an_uncertain_bar_restore_retains_exclusion_and_retries_exactly",
        },
    },
    Mutation {
        id: 919,
        file: "libs/pci-config/src/lib.rs",
        find: "pub const fn memory_window_size(low: u32, high: u32, width: MemoryWidth) -> Option<u64> {",
        replace: "pub const fn memory_window_size(low: u32, high: u32, width: bool) -> Option<u64> {",
        breaks: "PCI memory-window width becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 1004,
        file: "libs/pci-config/src/lib.rs",
        find: "const fn io_window_size(mask: u32) -> Option<u16> {",
        replace: "pub const fn io_window_size(mask: u32) -> Option<u16> {",
        breaks: "PCI I/O-window decoding regrows as speculative public arithmetic",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::address_space::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 1010,
        file: "libs/pci-config/src/lib.rs",
        find: "#[derive(Debug, PartialEq, Eq)]\n#[must_use = \"dropping function authority permanently retains its claim\"]\npub struct FunctionAuthority {",
        replace: "#[derive(Debug, Clone, PartialEq, Eq)]\n#[must_use = \"dropping function authority permanently retains its claim\"]\npub struct FunctionAuthority {",
        breaks: "pure PCI function authority can be cloned",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        id: 1011,
        file: "libs/pci-config/src/lib.rs",
        find: "#[derive(Debug, PartialEq, Eq)]\n#[must_use = \"dropping function authority permanently retains its claim\"]\npub struct FunctionAuthority {",
        replace: "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n#[must_use = \"dropping function authority permanently retains its claim\"]\npub struct FunctionAuthority {",
        breaks: "pure PCI function authority can be copied",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        id: 1014,
        file: "libs/pci-config/src/lib.rs",
        find: "struct CommandLeases<const N: usize> {",
        replace: "pub struct CommandLeases<const N: usize> {",
        breaks: "callers can substitute a parallel PCI command-lease registry",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        id: 1015,
        file: "libs/pci-config/src/lib.rs",
        find: "    fn release(&mut self, authority: FunctionAuthority) -> Result<(), FunctionAuthority> {",
        replace: "    pub fn release(&mut self, authority: FunctionAuthority) -> Result<(), FunctionAuthority> {",
        breaks: "callers can release PCI function authority without settling its live subleases",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        id: 1274,
        file: "libs/pci-config/src/lib.rs",
        find: "    let at = state.lease.command_address();\n    let current = access(at, CommandAccess::Read).ok_or(EnableError::Access)? as u16;\n    if current != state.installed {",
        replace: "    let at = state.lease.command_address();\n    let current = access(at, CommandAccess::Read).unwrap_or(u32::from(state.installed)) as u16;\n    if current != state.installed {",
        breaks: "an unreadable bus-master upgrade borrows its expected command word",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_bus_master_upgrade_access_refusal_preserves_its_owner",
        },
    },
    Mutation {
        id: 1275,
        file: "libs/pci-config/src/lib.rs",
        find: "        Err(error) => {\n            if error != EnableError::Restore {\n                state.phase = CommandPhase::Installed;\n            }\n            Err(error)\n        }",
        replace: "        Err(error) => {\n            state.phase = CommandPhase::Installed;\n            Err(error)\n        }",
        breaks: "an uncertain bus-master rollback loses its retry phase",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::failed_bus_master_upgrade_rollback_never_loses_cleanup_authority",
        },
    },
    Mutation {
        id: 1276,
        file: "libs/pci-config/src/lib.rs",
        find: "    if !leases.holds_function(function, state.lease) || state.phase != CommandPhase::Installed {",
        replace: "    if state.phase != CommandPhase::Installed {",
        breaks: "a command owner upgrades bus mastering through another function",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::bus_master_upgrade_rejects_another_function_before_access",
        },
    },
    Mutation {
        id: 1277,
        file: "libs/pci-config/src/lib.rs",
        find: "    admit_bus_master_with_leases(&mut functions.commands, authority.function, state, access)",
        replace: "    admit_bus_master_with_leases(&mut functions.commands, state.lease.function, state, access)",
        breaks: "bus-master admission substitutes the command owner's function for the mapped BAR",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::mapped_bus_master_admission_requires_its_bar_and_command_owner",
        },
    },
    Mutation {
        id: 1286,
        file: "libs/pci-config/src/lib.rs",
        find: "    if !functions.holds(function) || bars.function != Some(function.lease) {",
        replace: "    if !functions.holds(function) {",
        breaks: "a function can reserve a BAR probed through another function generation",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bar_mapping_authority_is_linear_and_bound_to_its_probe_function",
        },
    },
    Mutation {
        id: 1287,
        file: "libs/pci-config/src/lib.rs",
        find: "    if Bar::decode(low, high) != Some(authority.window.bar) {",
        replace: "    if false {",
        breaks: "memory decode ignores a changed live BAR identity",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::changed_or_foreign_bar_identity_performs_zero_command_writes",
        },
    },
    Mutation {
        id: 1288,
        file: "libs/pci-config/src/lib.rs",
        find: "const PROBE_DISABLED: u16 = PROBE_DECODE | command::BUS_MASTER;",
        replace: "const PROBE_DISABLED: u16 = PROBE_DECODE;",
        breaks: "BAR probing leaves inherited bus mastering enabled",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::inherited_bus_master_stays_off_until_the_staged_command_retires",
        },
    },
    Mutation {
        id: 1289,
        file: "libs/pci-config/src/lib.rs",
        find: "        Ok(CommandState {\n            lease: probe.0,",
        replace: "        let _released = self.release(probe.0);\n        Ok(CommandState {\n            lease: probe.0,",
        breaks: "a staged probe releases command ownership before retirement",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::inherited_bus_master_stays_off_until_the_staged_command_retires",
        },
    },
    Mutation {
        id: 1290,
        file: "libs/pci-config/src/lib.rs",
        find: "                assert!(\n                    leases.finish_probe(authority, probe).is_ok(),\n                    \"a restored BAR-probe refusal lost its exact reservation\"\n                );",
        replace: "                let _retained = probe;",
        breaks: "a clean BAR-probe refusal retains its function sublease after exact rollback",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::refused_bus_master_clear_releases_only_after_exact_rollback",
        },
    },
    Mutation {
        id: 1291,
        file: "libs/pci-config/src/lib.rs",
        find: "        if memory_type == Self::BELOW_ONE_MEGABYTE {\n            return None;\n        }",
        replace: "        if false && memory_type == Self::BELOW_ONE_MEGABYTE {\n            return None;\n        }",
        breaks: "the obsolete below-one-megabyte BAR encoding is accepted as ordinary memory",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_memory_bar_type_encoding_has_one_exact_outcome",
        },
    },
    Mutation {
        id: 1292,
        file: "libs/pci-config/src/lib.rs",
        find: "        if memory_type == Self::RESERVED {\n            return None;\n        }",
        replace: "        if false && memory_type == Self::RESERVED {\n            return None;\n        }",
        breaks: "the reserved memory BAR encoding is accepted as ordinary memory",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_memory_bar_type_encoding_has_one_exact_outcome",
        },
    },
    Mutation {
        id: 1293,
        file: "libs/pci-config/src/lib.rs",
        find: "            Bar::decode(original[low], 0)?,",
        replace: "            Bar::decode(original[low], 0).unwrap_or(Bar::Absent),",
        breaks: "an unsupported memory BAR reaches sizing writes",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::unsupported_memory_bar_types_refuse_before_any_probe_write",
        },
    },
    Mutation {
        id: 1294,
        file: "libs/pci-config/src/lib.rs",
        find: "    let low = read(low_at, authority.index).ok_or(BarIdentityError::Access)?;",
        replace: "    let low = read(low_at, authority.index).ok_or(BarIdentityError::Access)? & !Bar::TYPE_MASK;",
        breaks: "a type-only live BAR change is erased before identity verification",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_type_only_bar_change_performs_zero_command_writes",
        },
    },
    Mutation {
        id: 1295,
        file: "libs/pci-config/src/lib.rs",
        find: "    let low = read(low_at, authority.index).ok_or(BarIdentityError::Access)?;\n    let high = match authority.window.bar {\n        Bar::Memory { wide: true, .. } => {\n            let high_offset = low_offset.checked_add(4).ok_or(BarIdentityError::Stale)?;\n            if authority.index + 1 >= function.bar_registers() {\n                return Err(BarIdentityError::Stale);\n            }\n            let high_at = function\n                .address()\n                .at(high_offset)\n                .ok_or(BarIdentityError::Stale)?;\n            read(high_at, authority.index + 1).ok_or(BarIdentityError::Access)?\n        }\n        Bar::Memory { wide: false, .. } => 0,\n        Bar::Io { .. } | Bar::Absent => return Err(BarIdentityError::Stale),\n    };",
        replace: "    let high = match authority.window.bar {\n        Bar::Memory { wide: true, .. } => {\n            let high_offset = low_offset.checked_add(4).ok_or(BarIdentityError::Stale)?;\n            if authority.index + 1 >= function.bar_registers() {\n                return Err(BarIdentityError::Stale);\n            }\n            let high_at = function\n                .address()\n                .at(high_offset)\n                .ok_or(BarIdentityError::Stale)?;\n            read(high_at, authority.index + 1).ok_or(BarIdentityError::Access)?\n        }\n        Bar::Memory { wide: false, .. } => 0,\n        Bar::Io { .. } | Bar::Absent => return Err(BarIdentityError::Stale),\n    };\n    let low = read(low_at, authority.index).ok_or(BarIdentityError::Access)?;",
        breaks: "a wide BAR reads its high half before its low half",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::exact_wide_bar_reads_precede_the_first_decode_write",
        },
    },
    Mutation {
        id: 1297,
        file: "libs/pci-config/src/lib.rs",
        find: "#[derive(Debug, PartialEq, Eq)]\n#[must_use = \"a BAR authority must remain with its mapping or be returned\"]\npub struct BarAuthority {",
        replace: "#[derive(Debug, Clone, PartialEq, Eq)]\n#[must_use = \"a BAR authority must remain with its mapping or be returned\"]\npub struct BarAuthority {",
        breaks: "one BAR reservation can be replayed into multiple mappings",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci::pci_bar_mapping_requires_exact_linear_authority",
        },
    },
    Mutation {
        id: 1386,
        file: "libs/pci-config/src/lib.rs",
        find: "            Self::Access(reason) => write!(f, \"configuration access failed: {reason}\"),",
        replace: "            Self::Access(reason) => core::fmt::Display::fmt(reason, f),",
        breaks: "a BAR-probe refusal hides the configuration-access operation that failed",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_probe_error_describes_and_exposes_its_cause",
        },
    },
    Mutation {
        id: 1388,
        file: "libs/pci-config/src/lib.rs",
        find: "#[derive(Debug, PartialEq, Eq)]\n#[must_use = \"a refused function release returns its authority for retry\"]\npub struct FunctionReleaseFailure {",
        replace: "#[derive(Debug, PartialEq, Eq)]\npub struct FunctionReleaseFailure {",
        breaks: "a linear failure wrapper can be discarded without a compiler warning",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::errors::public_failure_wrappers_are_must_use",
        },
    },
    Mutation {
        id: 1409,
        file: "libs/pci-config/src/lib.rs",
        find: "pub struct FunctionAuthority {\n    lease: FunctionLease,\n    registers: usize,\n}\n\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\nstruct FunctionLease {",
        replace: "pub struct FunctionAuthority {\n    pub lease: FunctionLease,\n    registers: usize,\n}\n\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub struct FunctionLease {",
        breaks: "pure PCI function authority exposes its private registry lease",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        id: 1475,
        file: "libs/pci-config/src/lib.rs",
        find: "impl FunctionAuthority {\n",
        replace: "impl FunctionAuthority {\n    /// Reissue this authority for another holder.\n    pub const fn reissue(&self) -> Self {\n        Self {\n            lease: self.lease,\n            registers: self.registers,\n        }\n    }\n\n",
        breaks: "pure PCI function authority exposes a safe reissue door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci::pci_function_authority_has_audited_mints_and_no_duplication_door",
        },
    },
    Mutation {
        id: 1477,
        file: "libs/pci-config/src/lib.rs",
        find: "    /// BAR register slots proved by the claimed function's header layout.\n    pub const fn bar_registers(&self) -> usize {\n        self.registers\n    }\n",
        replace: "    /// BAR register slots proved by the claimed function's header layout.\n    pub const fn bar_registers(&self) -> usize {\n        self.registers\n    }\n\n    /// Point this authority at another function.\n    pub fn retarget(&mut self, at: Address) {\n        self.lease.at = at.function_base();\n    }\n",
        breaks: "pure PCI function authority can be retargeted through a mutable borrow",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci::pci_function_authority_has_audited_mints_and_no_duplication_door",
        },
    },
    Mutation {
        id: 1550,
        file: "libs/pci-config/src/lib.rs",
        find: "    if authority.function.at.function_base() != isolated.function_base() {\n        return Err(EnableError::ForeignDomain);\n    }\n",
        replace: "    let _isolated = isolated;\n",
        breaks: "a domain established for another function admits this one's bus mastering",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::mapped_bus_master_admission_requires_its_bar_and_command_owner",
        },
    },
    Mutation {
        id: 1569,
        file: "libs/pci-config/src/lib.rs",
        find: "pub const fn subordinate_bus(bits: u32) -> u8 {\n    (bits >> 16) as u8\n}",
        replace: "pub const fn subordinate_bus(bits: u32) -> u8 {\n    (bits >> 24) as u8\n}",
        breaks: "a bridge's subordinate bus is read from its latency-timer byte",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bridge_subordinate_bus_is_read_from_its_own_byte",
        },
    },
    Mutation {
        id: 1651,
        file: "libs/pci-config/src/lib.rs",
        find: "            state.before &= !command::BUS_MASTER;",
        replace: "            let _safe_before = state.before & !command::BUS_MASTER;",
        breaks: "a rejected DMA lease restores firmware's inherited bus-master enable bit",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::retirement_disables_and_does_not_restore_inherited_bus_master",
        },
    },
    Mutation {
        id: 1652,
        file: "libs/pci-config/src/lib.rs",
        find: "    if current != state.installed && pending != Some(current) {\n        return Err(EnableError::Changed(current));\n    }\n\n    // `current` is now one of the command owner's exact recorded words. Make",
        replace: "    if current != state.installed {\n        return Err(EnableError::Changed(current));\n    }\n\n    // `current` is now one of the command owner's exact recorded words. Make",
        breaks: "an interrupted bus-master transition cannot settle through its recorded pending word",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::an_uncertain_retirement_transition_can_settle_disabled",
        },
    },
    Mutation {
        id: 1658,
        file: "libs/pci-config/src/lib.rs",
        find: "    disable_bus_master_for_retirement_with_leases(\n        &mut functions.commands,\n        authority.lease,\n        state,\n        access,\n    )",
        replace: "    disable_bus_master_for_retirement_with_leases(\n        &mut functions.commands,\n        state.lease.function,\n        state,\n        access,\n    )",
        breaks: "a foreign function authority can fence a command state issued to another function",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::configuration_cycles_require_the_issuing_function_registry",
        },
    },
    Mutation {
        id: 1659,
        file: "libs/pci-config/src/lib.rs",
        find: "    match change_command(at, current, wanted, &mut access) {\n        Ok(()) => {\n            state.before &= !command::BUS_MASTER;",
        replace: "    state.before &= !command::BUS_MASTER;\n    match change_command(at, current, wanted, &mut access) {\n        Ok(()) => {",
        breaks: "a refused retirement transition permanently weakens the captured restore word before verification",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_retirement_access_refusal_retains_exact_retry_authority",
        },
    },
    Mutation {
        id: 1671,
        file: "libs/pci-config/src/lib.rs",
        find: "    // `begin_command` verified the installed word exactly. From this point a\n    // later restoration is deliberately safe rather than firmware-exact.\n    state.before &= !command::BUS_MASTER;",
        replace: "    // The inherited BME bit was incorrectly left in the restoration target.",
        breaks: "requester-fence release restores inherited bus mastering",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::unit_requester_fence_withholds_and_never_restores_inherited_bus_master",
        },
    },
    Mutation {
        id: 1672,
        file: "libs/pci-config/src/lib.rs",
        find: "    if current != state.installed {\n        return Err(EnableError::Changed(current));\n    }\n    if current & command::BUS_MASTER != 0 {",
        replace: "    if current & command::BUS_MASTER != 0 {",
        breaks: "fresh fence verification accepts unrelated command-word drift",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::requester_fence_and_fresh_verification_retain_exact_failure_authority",
        },
    },
    Mutation {
        id: 1684,
        file: "libs/pci-config/src/lib.rs",
        find: "        if *slot != 0 && *slot != at.bits() {\n            return Err(TopologyError::DuplicateParent {\n                bus: secondary,\n                first: Address(*slot),\n                second: at,\n            });\n        }",
        replace: "        let _competing_parent = *slot;",
        breaks: "a PCI bus can acquire two different direct parent bridges",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::duplicate_unrooted_and_contradictory_functions_refuse_topology",
        },
    },
    Mutation {
        id: 1685,
        file: "libs/pci-config/src/lib.rs",
        find: "            if at.bus() != 0 && !self.bus_reaches_root(at.bus()) {\n                return Err(TopologyError::UnrootedFunction { at });\n            }",
        replace: "            let _unrooted_function = at;",
        breaks: "a present DMA requester can exist outside the proved bus-zero parent chain",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::duplicate_unrooted_and_contradictory_functions_refuse_topology",
        },
    },
    Mutation {
        id: 1686,
        file: "libs/pci-config/src/lib.rs",
        find: "            if secondary < parent_secondary\n                || self.subordinate[usize::from(secondary)]\n                    > self.subordinate[usize::from(parent_secondary)]\n            {\n                return Err(TopologyError::ChildEscapesParent { child, parent });\n            }",
        replace: "            let _unbounded_child_range = parent_secondary;",
        breaks: "a child bridge forwards buses outside its direct parent's admitted range",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::child_escape_and_unrelated_overlap_refuse_topology",
        },
    },
    Mutation {
        id: 1687,
        file: "libs/pci-config/src/lib.rs",
        find: "                if overlap\n                    && !self.bridge_is_ancestor(first, second)\n                    && !self.bridge_is_ancestor(second, first)\n                {\n                    return Err(TopologyError::UnrelatedOverlap { first, second });\n                }",
        replace: "                let _unrelated_overlap = overlap;",
        breaks: "unrelated PCI bridge branches may advertise overlapping bus intervals",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::child_escape_and_unrelated_overlap_refuse_topology",
        },
    },
    Mutation {
        id: 1692,
        file: "libs/pci-config/src/lib.rs",
        find: "        let Some(bits) = bus_numbers else {\n            return Err(TopologyError::MissingBusNumbers { at });\n        };",
        replace: "        let bits = bus_numbers.unwrap_or(0);",
        breaks: "an unreadable bridge bus-number register is fabricated as zero",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::unreadable_and_invalid_bridge_bus_numbers_refuse_topology",
        },
    },
    Mutation {
        id: 1693,
        file: "libs/pci-config/src/lib.rs",
        find: "        if primary != at.bus() || secondary <= at.bus() || secondary > subordinate {",
        replace: "        if secondary <= at.bus() || secondary > subordinate {",
        breaks: "a bridge may claim a primary bus different from the bus containing it",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::unreadable_and_invalid_bridge_bus_numbers_refuse_topology",
        },
    },
    Mutation {
        id: 1694,
        file: "libs/pci-config/src/lib.rs",
        find: "        if primary != at.bus() || secondary <= at.bus() || secondary > subordinate {",
        replace: "        if primary != at.bus() || secondary > subordinate {",
        breaks: "a bridge may forward to its own bus instead of a strict child bus",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::unreadable_and_invalid_bridge_bus_numbers_refuse_topology",
        },
    },
    Mutation {
        id: 1695,
        file: "libs/pci-config/src/lib.rs",
        find: "        if primary != at.bus() || secondary <= at.bus() || secondary > subordinate {",
        replace: "        if primary != at.bus() || secondary <= at.bus() {",
        breaks: "a bridge may advertise a secondary bus above its subordinate limit",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::unreadable_and_invalid_bridge_bus_numbers_refuse_topology",
        },
    },
    Mutation {
        id: 1696,
        file: "libs/pci-config/src/lib.rs",
        find: "        if class.is_bridge() != header.is_bridge() {\n            return Err(TopologyError::BridgeIdentity { at });\n        }",
        replace: "        let _contradictory_bridge_identity = header.is_bridge();",
        breaks: "a function's bridge class and header layout may contradict each other",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::duplicate_unrooted_and_contradictory_functions_refuse_topology",
        },
    },
    Mutation {
        id: 1697,
        file: "libs/pci-config/src/lib.rs",
        find: "pub const fn primary_bus(bits: u32) -> u8 {\n    bits as u8\n}",
        replace: "pub const fn primary_bus(bits: u32) -> u8 {\n    (bits >> 8) as u8\n}",
        breaks: "the bridge primary bus is decoded from the secondary byte",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bridge_primary_bus_is_read_from_its_own_byte",
        },
    },
    Mutation {
        id: 1698,
        file: "libs/pci-config/src/lib.rs",
        find: "        self.bridge_range(root).is_some()\n            && self.contains_function(target)\n            && self.bridge_is_ancestor(root, target)",
        replace: "        self.bridge_range(root).is_some() && self.contains_function(target)",
        breaks: "any present target is treated as below any bridge regardless of its parent chain",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_sealed_topology_binds_presence_ranges_and_ancestry",
        },
    },
    Mutation {
        id: 1699,
        file: "libs/pci-config/src/lib.rs",
        find: "        * usize::from(FUNCTIONS)\n        + usize::from(at.function())",
        replace: "        * usize::from(FUNCTIONS)",
        breaks: "the sealed presence bitmap aliases all functions of one PCI device",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_sealed_topology_binds_presence_ranges_and_ancestry",
        },
    },
    Mutation {
        id: 1700,
        file: "libs/pci-config/src/lib.rs",
        find: "                    subordinate: self.subordinate[usize::from(secondary)],",
        replace: "                    subordinate: secondary,",
        breaks: "a sealed bridge query truncates its forwarded range to the secondary bus",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_sealed_topology_binds_presence_ranges_and_ancestry",
        },
    },
    Mutation {
        id: 1702,
        file: "libs/pci-config/src/lib.rs",
        find: "        self.sealed = true;\n        Ok(())",
        replace: "        self.sealed = false;\n        Ok(())",
        breaks: "a fully validated PCI topology never becomes queryable as sealed",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_sealed_topology_binds_presence_ranges_and_ancestry",
        },
    },
    Mutation {
        id: 1823,
        file: "libs/pci-config/src/lib.rs",
        find: "    for bus in 0..=u8::MAX {",
        replace: "    for bus in 0..u8::MAX {",
        breaks: "the entry fence omits every requester on the final PCI bus",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::the_handoff_fence_exhausts_the_segment_and_preserves_command_bits",
        },
    },
    Mutation {
        id: 1824,
        file: "libs/pci-config/src/lib.rs",
        find: "                let expected = before & !command::BUS_MASTER;",
        replace: "                let expected = before;",
        breaks: "the entry fence inventories inherited bus masters without disabling them",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::the_handoff_fence_exhausts_the_segment_and_preserves_command_bits",
        },
    },
    Mutation {
        id: 1825,
        file: "libs/pci-config/src/lib.rs",
        find: "                let retained_class = if found != expected {",
        replace: "                let retained_class = if false && found != expected {",
        breaks: "an ignored bus-master clear is treated as a completed entry fence",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_handoff_fence_uncertainty_refuses_the_boundary",
        },
    },
    Mutation {
        id: 1826,
        file: "libs/pci-config/src/lib.rs",
        find: "                if current != identity {",
        replace: "                if false && current != identity {",
        breaks: "a requester identity change during the entry fence is accepted as static topology",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_handoff_fence_uncertainty_refuses_the_boundary",
        },
    },
    Mutation {
        id: 1841,
        file: "libs/pci-config/src/lib.rs",
        find: "            && at.bus() == 0\n            && at.device() == 0\n            && at.function() == 0",
        replace: "            && at.bus() == 0",
        breaks: "another VMware BDF can impersonate the one qualified non-requester host bridge",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_440bx_host_bridge_may_retain_its_non_requester_command",
        },
    },
    Mutation {
        id: 1842,
        file: "libs/pci-config/src/lib.rs",
        find: "            && identity == VMWARE_440BX_IDENTITY",
        replace: "            && identity != 0",
        breaks: "a foreign VMware function identity can retain inherited bus-master-looking state",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_440bx_host_bridge_may_retain_its_non_requester_command",
        },
    },
    Mutation {
        id: 1843,
        file: "libs/pci-config/src/lib.rs",
        find: "            && class & CLASS_WITHOUT_REVISION == HOST_BRIDGE_CLASS",
        replace: "            && class & CLASS_WITHOUT_REVISION != 0",
        breaks: "a non-host-bridge VMware class can enter the non-requester exception",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_440bx_host_bridge_may_retain_its_non_requester_command",
        },
    },
    Mutation {
        id: 1844,
        file: "libs/pci-config/src/lib.rs",
        find: "            && before == VMWARE_HOST_COMMAND",
        replace: "            && before & command::BUS_MASTER != 0",
        breaks: "an unexpected VMware host command is accepted before the BME-clear attempt",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_440bx_host_bridge_may_retain_its_non_requester_command",
        },
    },
    Mutation {
        id: 1845,
        file: "libs/pci-config/src/lib.rs",
        find: "            && found == VMWARE_HOST_COMMAND)",
        replace: "            && found & command::BUS_MASTER != 0)",
        breaks: "an unexpected VMware host command readback is accepted as the exact exception",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_440bx_host_bridge_may_retain_its_non_requester_command",
        },
    },
    Mutation {
        id: 1846,
        file: "libs/pci-config/src/lib.rs",
        find: "                    if current_class != class {",
        replace: "                    if false && current_class != class {",
        breaks: "the qualified host-bridge class can change after its refused command write",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_440bx_host_bridge_may_retain_its_non_requester_command",
        },
    },
    Mutation {
        id: 1848,
        file: "libs/pci-config/src/lib.rs",
        find: "                            retained_non_requesters += 1;\n                            retained_non_requester = Some(authority);",
        replace: "                            disabled += 1;\n                            retained_non_requesters += 1;\n                            retained_non_requester = Some(authority);",
        breaks: "an attempted but refused VMware host-bridge clear is counted as a disabled requester",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_440bx_host_bridge_may_retain_its_non_requester_command",
        },
    },
    Mutation {
        id: 1854,
        file: "libs/pci-config/src/lib.rs",
        find: "    fn classify_non_requester(",
        replace: "    pub fn classify_non_requester(",
        breaks: "callers outside the exhaustive entry fence can mint non-requester authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::entry::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1855,
        file: "libs/pci-config/src/lib.rs",
        find: "        if at != authority.at",
        replace: "        if false && at != authority.at",
        breaks: "the later topology can spend entry authority on another PCI function",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_440bx_host_bridge_may_retain_its_non_requester_command",
        },
    },
    Mutation {
        id: 1856,
        file: "libs/pci-config/src/lib.rs",
        find: "            || identity != authority.identity",
        replace: "            || false && identity != authority.identity",
        breaks: "the later topology accepts a changed non-requester identity",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_440bx_host_bridge_may_retain_its_non_requester_command",
        },
    },
    Mutation {
        id: 1857,
        file: "libs/pci-config/src/lib.rs",
        find: "            || class_bits != authority.class",
        replace: "            || false && class_bits != authority.class",
        breaks: "the later topology accepts a changed non-requester class",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_440bx_host_bridge_may_retain_its_non_requester_command",
        },
    },
    Mutation {
        id: 1858,
        file: "libs/pci-config/src/lib.rs",
        find: "            || command != authority.command",
        replace: "            || false && command != authority.command",
        breaks: "the later topology accepts changed command state for the non-requester",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_440bx_host_bridge_may_retain_its_non_requester_command",
        },
    },
    Mutation {
        id: 1861,
        file: "libs/pci-config/src/lib.rs",
        find: "#[derive(Debug)]\n#[must_use = \"qualified non-requester evidence must constrain requester inventory\"]",
        replace: "#[derive(Debug, Clone, Copy)]\n#[must_use = \"qualified non-requester evidence must constrain requester inventory\"]",
        breaks: "the exact non-requester authority can be duplicated and spent twice",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::entry::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1865,
        file: "libs/pci-config/src/lib.rs",
        find: "    pub const fn retained_non_requesters(&self) -> u32 {\n        self.retained_non_requesters\n    }",
        replace: "    pub const fn retained_non_requesters(&self) -> u32 {\n        self.retained_non_requester.is_some() as u32\n    }",
        breaks: "taking the qualified non-requester token erases the fence report's historical retained count",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_440bx_host_bridge_may_retain_its_non_requester_command",
        },
    },
    Mutation {
        id: 2272,
        file: "libs/pci-config/src/lib.rs",
        find: "            && (at.bus(), at.device(), at.function()) == (0, 7, 0)",
        replace: "            && at.device() == 7",
        breaks: "another VMware function number can impersonate the quiesced PIIX4 ISA bridge",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_piix4_isa_bridge_is_quiesced_and_stays_a_requester",
        },
    },
    Mutation {
        id: 2273,
        file: "libs/pci-config/src/lib.rs",
        find: "            && identity == VMWARE_PIIX4_ISA_IDENTITY",
        replace: "            && identity != 0",
        breaks: "a foreign identity at the ISA bridge address is quiesced instead of refused",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_piix4_isa_bridge_is_quiesced_and_stays_a_requester",
        },
    },
    Mutation {
        id: 2274,
        file: "libs/pci-config/src/lib.rs",
        find: "            && class & ISA_CLASS_MASK == ISA_BRIDGE_CLASS",
        replace: "            && class & ISA_CLASS_MASK != 0",
        breaks: "a non-ISA-bridge class at the PIIX4 address enters the quiesce exception",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_piix4_isa_bridge_is_quiesced_and_stays_a_requester",
        },
    },
    Mutation {
        id: 2275,
        file: "libs/pci-config/src/lib.rs",
        find: "            && before == VMWARE_ISA_COMMAND",
        replace: "            && before & command::BUS_MASTER != 0",
        breaks: "an unexpected PIIX4 command before the clear is accepted for quiesce",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_piix4_isa_bridge_is_quiesced_and_stays_a_requester",
        },
    },
    Mutation {
        id: 2276,
        file: "libs/pci-config/src/lib.rs",
        find: "            && found == VMWARE_ISA_COMMAND",
        replace: "            && found & command::BUS_MASTER != 0",
        breaks: "an unexpected PIIX4 command readback is accepted for quiesce",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_piix4_isa_bridge_is_quiesced_and_stays_a_requester",
        },
    },
    Mutation {
        id: 2277,
        file: "libs/pci-config/src/lib.rs",
        find: "                        let status = access(at, HandoffAccess::QuiesceIsaDma).ok_or(",
        replace: "                        let status = Some(0u32).ok_or(",
        breaks: "the ISA bridge is admitted without ever masking its 8237 controllers",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_piix4_isa_bridge_is_quiesced_and_stays_a_requester",
        },
    },
    Mutation {
        id: 2278,
        file: "libs/pci-config/src/lib.rs",
        find: "                        if status & ISA_DMA_PENDING_REQUESTS != 0 {",
        replace: "                        if false && status & ISA_DMA_PENDING_REQUESTS != 0 {",
        breaks: "a pending 8237 request after masking still mints quiesce authority",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_piix4_isa_bridge_is_quiesced_and_stays_a_requester",
        },
    },
    Mutation {
        id: 2279,
        file: "libs/pci-config/src/lib.rs",
        find: "        if observed != bound {",
        replace: "        if false && observed != bound {",
        breaks: "the later snapshot accepts a changed function for the quiesce authority",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_piix4_isa_bridge_is_quiesced_and_stays_a_requester",
        },
    },
    Mutation {
        id: 2280,
        file: "libs/pci-config/src/lib.rs",
        find: "        self.quiesced = Some(QuiescedRequester {",
        replace: "        let index = function_index(at);\n        self.requesters[index / u64::BITS as usize] &= !(1 << (index % u64::BITS as usize));\n        self.quiesced = Some(QuiescedRequester {",
        breaks: "the quiesced ISA requester silently leaves the VT-d requester inventory",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_piix4_isa_bridge_is_quiesced_and_stays_a_requester",
        },
    },
    Mutation {
        id: 2281,
        file: "libs/pci-config/src/lib.rs",
        find: "        self.quiesced\n            .filter(|record| record.at == at.function_base())",
        replace: "        self.quiesced",
        breaks: "every function inherits the quiesced requester's fence exemption",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_piix4_isa_bridge_is_quiesced_and_stays_a_requester",
        },
    },
    Mutation {
        id: 2282,
        file: "libs/pci-config/src/lib.rs",
        find: "    pub fn quiesced_requester(&self, at: Address) -> Option<QuiescedRequester> {\n        if !self.sealed {\n            return None;\n        }",
        replace: "    pub fn quiesced_requester(&self, at: Address) -> Option<QuiescedRequester> {",
        breaks: "an unsealed snapshot answers the quiesced-requester query",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::only_the_exact_vmware_piix4_isa_bridge_is_quiesced_and_stays_a_requester",
        },
    },
    Mutation {
        id: 2283,
        file: "libs/pci-config/src/lib.rs",
        find: "pub const FUNCTION_AUTHORITY_LIMIT: usize = 64;",
        replace: "pub const FUNCTION_AUTHORITY_LIMIT: usize = 12;",
        breaks: "function authority capacity falls below VMware's 42 live requesters",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::the_captured_vmware_segment_fences_and_fits_the_function_authority_limit",
        },
    },
    Mutation {
        id: 2315,
        file: "libs/pci-config/src/lib.rs",
        find: "pub const ISA_DMA_PENDING_REQUESTS: u32 = 0xf0f0;",
        replace: "pub const ISA_DMA_PENDING_REQUESTS: u32 = 0xd0d0;",
        breaks: "one request bit in each ISA DMA status word escapes the quiet predicate",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma::isa::isa_dma_adapter_clears_masks_and_rechecks_both_controllers",
        },
    },
    Mutation {
        id: 2338,
        file: "libs/pci-config/src/lib.rs",
        find: "            self.observation_failure = Some(error);",
        replace: "            let _ = error;\n            self.observation_failure = None;",
        breaks: "The first locally rejected observation cannot be resealed as structurally complete.",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_ordinary_observation_error_poisoned_until_reset",
        },
    },
    Mutation {
        id: 2339,
        file: "libs/pci-config/src/lib.rs",
        find: "    fn begin_observation(&mut self) -> Result<(), TopologyError> {\n        self.sealed = false;",
        replace: "    fn begin_observation(&mut self) -> Result<(), TopologyError> {",
        breaks: "Immediately after the failed ordinary observation, prior sealed authority queries must be hidden; refusal at seal alone is insufficient.",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_ordinary_observation_error_poisoned_until_reset",
        },
    },
    Mutation {
        id: 2340,
        file: "libs/pci-config/src/lib.rs",
        find: "        match self.observation_failure {\n            Some(error) => Err(error),\n            None => Ok(()),\n        }",
        replace: "        match self.observation_failure {\n            Some(_error) => Ok(()),\n            None => Ok(()),\n        }",
        breaks: "A later valid observation must still return the original error; a different invalid observation cannot replace it.",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_ordinary_observation_error_poisoned_until_reset",
        },
    },
    Mutation {
        id: 2341,
        file: "libs/pci-config/src/lib.rs",
        find: "    pub fn observe_non_requester(\n        &mut self,\n        authority: QualifiedNonRequester,\n        observation: FunctionObservation,\n    ) -> Result<(), TopologyError> {\n        self.begin_observation()?;",
        replace: "    pub fn observe_non_requester(\n        &mut self,\n        authority: QualifiedNonRequester,\n        observation: FunctionObservation,\n    ) -> Result<(), TopologyError> {",
        breaks: "A tuple mismatch before delegated observe must immediately revoke the prior seal; the ordinary door cannot mask omission of this special-door entry.",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_failed_non_requester_observation_revokes_the_seal_and_retains_its_error",
        },
    },
    Mutation {
        id: 2342,
        file: "libs/pci-config/src/lib.rs",
        find: "    pub fn observe_quiesced_requester(\n        &mut self,\n        authority: QuiescedIsaRequester,\n        observation: FunctionObservation,\n    ) -> Result<(), TopologyError> {\n        self.begin_observation()?;",
        replace: "    pub fn observe_quiesced_requester(\n        &mut self,\n        authority: QuiescedIsaRequester,\n        observation: FunctionObservation,\n    ) -> Result<(), TopologyError> {",
        breaks: "A tuple mismatch before delegation must immediately hide the prior quiesced-requester record and seal.",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_failed_quiesced_observation_revokes_the_seal_and_retains_its_error",
        },
    },
    Mutation {
        id: 2343,
        file: "libs/pci-config/src/lib.rs",
        find: "            return self.retain_observation_failure(Err(\n                TopologyError::QualifiedNonRequesterChanged { at },\n            ));",
        replace: "            return Err(TopologyError::QualifiedNonRequesterChanged { at });",
        breaks: "After the tuple refusal, reseal must return that same error rather than reauthorize the previous snapshot.",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_failed_non_requester_observation_revokes_the_seal_and_retains_its_error",
        },
    },
    Mutation {
        id: 2344,
        file: "libs/pci-config/src/lib.rs",
        find: "            return self\n                .retain_observation_failure(Err(TopologyError::QuiescedRequesterChanged { at }));",
        replace: "            return Err(TopologyError::QuiescedRequesterChanged { at });",
        breaks: "After the tuple refusal, reseal must retain that error and keep the previous quiesce authority inaccessible.",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_failed_quiesced_observation_revokes_the_seal_and_retains_its_error",
        },
    },
    Mutation {
        id: 2345,
        file: "libs/pci-config/src/lib.rs",
        find: "        if let Some(error) = self.observation_failure {\n            return Err(error);\n        }",
        replace: "",
        breaks: "A structurally valid partial snapshot cannot gain a seal after the earlier observation failure, including repeated seal attempts.",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_ordinary_observation_error_poisoned_until_reset",
        },
    },
    Mutation {
        id: 2346,
        file: "libs/pci-config/src/lib.rs",
        find: "        self.quiesced = None;\n        self.observation_failure = None;\n        self.sealed = false;",
        replace: "        self.quiesced = None;\n        self.sealed = false;",
        breaks: "After reset, a fresh valid root endpoint must observe and seal successfully without the prior error or prior function.",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_ordinary_observation_error_poisoned_until_reset",
        },
    },
    Mutation {
        id: 2347,
        file: "libs/pci-config/src/lib.rs",
        find: "        self.observe(at, class, header, bus_numbers)?;",
        replace: "        self.observe_function(at, class, header, bus_numbers)?;",
        breaks: "non-requester delegated validation retains poison",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::delegated_non_requester_errors_stay_poisoned_until_reset",
        },
    },
    Mutation {
        id: 2348,
        file: "libs/pci-config/src/lib.rs",
        find: "        self.observe(\n            at,\n            observation.class,\n            observation.header,\n            observation.bus_numbers,\n        )?;",
        replace: "        self.observe_function(\n            at,\n            observation.class,\n            observation.header,\n            observation.bus_numbers,\n        )?;",
        breaks: "quiesced-requester delegated validation retains poison",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::delegated_quiesced_requester_errors_stay_poisoned_until_reset",
        },
    },
    Mutation {
        id: 2455,
        file: "libs/pci-config/src/lib.rs",
        find: "if address >> 16 != 0 {\n                return None;\n            }",
        replace: "if false {\n                return None;\n            }",
        breaks: "an I/O BAR above the port space decodes as a low port",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_base_address_register_decodes_to_its_window",
        },
    },
    Mutation {
        id: 2456,
        file: "libs/pci-config/src/lib.rs",
        find: "    let mask = register_bits::u16_halves(mask).0 & !0x3;\n",
        replace: "    if mask >> 16 == 0 {\n        return None;\n    }\n    let mask = register_bits::u16_halves(mask).0 & !0x3;\n",
        breaks: "a 16-bit I/O decoder's hardwired-zero upper half is sized as a window too large for port space",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::an_io_window_size_is_a_port_count",
        },
    },
];
