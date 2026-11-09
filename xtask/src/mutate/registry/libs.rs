use super::{Caught, Mutation};

pub(super) const MUTATIONS: &[Mutation] = &[
    Mutation {
        id: 82,
        file: "libs/ring-proto/src/lib.rs",
        find: "    pub const fn is_consistent(&self) -> bool {\n        (self.reserved as u64) + (self.completion.occupied() as u64)\n            <= self.completion.capacity().entries() as u64\n    }",
        replace: "    pub const fn is_consistent(&self) -> bool {\n        true\n    }",
        breaks: "the ring reservation diagnostic cannot report a broken invariant",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "tests::the_consistency_oracle_refuses_an_over_reserved_ring",
        },
    },
    Mutation {
        id: 83,
        file: "libs/ring-proto/src/lib.rs",
        find: "    pub const fn tail(&self) -> u32 {\n        self.tail\n    }",
        replace: "    pub const fn tail(&self) -> u32 {\n        0\n    }",
        breaks: "the public tail accessor reports a slot-like constant instead of its counter",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "tests::counter_accessors_report_counts_not_masked_slots",
        },
    },
    Mutation {
        id: 84,
        file: "libs/ring-proto/src/lib.rs",
        find: "    pub const fn is_full(&self) -> bool {\n        self.occupied() == self.capacity.entries()\n    }",
        replace: "    pub const fn is_full(&self) -> bool {\n        self.occupied() != self.capacity.entries()\n    }",
        breaks: "the ring full-state predicate disagrees with its occupied count",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "occupancy_and_boundary_predicates_remain_consistent",
        },
    },
    Mutation {
        id: 85,
        file: "libs/ring-proto/src/lib.rs",
        find: "    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {\n        match *self {\n            Self::NotAPowerOfTwo(value) => write!(f, \"{value} is not a power of two\"),\n            Self::TooLarge(value) => write!(f, \"{value} entries is more than a ring may hold\"),\n        }\n    }",
        replace: "    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {\n        let _ = (self, f);\n        Ok(())\n    }",
        breaks: "capacity refusals lose the value and rule they report",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "tests::capacity_errors_name_the_refused_value_and_rule",
        },
    },
    Mutation {
        id: 86,
        file: "libs/frame-space/src/lib.rs",
        find: "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n        match *self {\n            Self::NotManaged(frame) => write!(f, \"frame {frame} is not managed memory\"),\n            Self::AlreadyFree(frame) => write!(f, \"frame {frame} is already free\"),\n        }\n    }",
        replace: "    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {\n        Ok(())\n    }",
        breaks: "frame release refusals lose the frame and ownership failure",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::release_errors_name_the_frame_and_ownership_failure",
        },
    },
    Mutation {
        id: 87,
        file: "libs/frame-space/src/lib.rs",
        find: "    pub const fn is_free(&self, frame: usize) -> bool {\n        frame < Self::FRAME_COUNT && self.free[frame / FRAMES_PER_WORD] & bit(frame) != 0\n    }",
        replace: "    pub const fn is_free(&self, frame: usize) -> bool {\n        frame <= Self::FRAME_COUNT && self.free[frame / FRAMES_PER_WORD] & bit(frame) != 0\n    }",
        breaks: "the bitmap ceiling is treated as a free frame index",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::membership_stops_at_the_bitmap_ceiling",
        },
    },
    Mutation {
        id: 88,
        file: "libs/frame-space/src/lib.rs",
        find: "    pub const fn is_managed(&self, frame: usize) -> bool {\n        frame < Self::FRAME_COUNT && self.managed[frame / FRAMES_PER_WORD] & bit(frame) != 0\n    }",
        replace: "    pub const fn is_managed(&self, frame: usize) -> bool {\n        frame <= Self::FRAME_COUNT && self.managed[frame / FRAMES_PER_WORD] & bit(frame) != 0\n    }",
        breaks: "the bitmap ceiling is treated as a managed frame index",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::membership_stops_at_the_bitmap_ceiling",
        },
    },
    Mutation {
        id: 89,
        file: "libs/frame-space/src/lib.rs",
        find: "        if count == 0 || count > Self::FRAME_COUNT {",
        replace: "        if count == 0 || count >= Self::FRAME_COUNT {",
        breaks: "an allocation spanning the exact frame domain is refused as oversized",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::allocation_count_distinguishes_the_complete_domain_from_too_many",
        },
    },
    Mutation {
        id: 90,
        file: "libs/frame-space/src/lib.rs",
        find: "    pub fn allocate(&mut self) -> Option<usize> {\n        self.allocate_contiguous(1, 1)\n    }",
        replace: "    pub fn allocate(&mut self) -> Option<usize> {\n        Some(0)\n    }",
        breaks: "a frame-allocation sweep never reaches exhaustion",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::a_sweep_of_the_whole_space_conserves_every_frame",
        },
    },
    Mutation {
        id: 91,
        file: "libs/frame-space/src/lib.rs",
        find: "        for _ in 0..=Self::FRAME_COUNT {",
        replace: "        for _ in 0..=0 {",
        breaks: "first-fit stops after the first obstructed candidate window",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::a_contiguous_run_skips_past_a_hole",
        },
    },
    Mutation {
        id: 92,
        file: "libs/frame-space/src/lib.rs",
        find: "            if end > to {",
        replace: "            if end == to {",
        breaks: "first-fit may cross the search interval's exclusive upper frame",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::search_treats_its_upper_frame_as_exclusive",
        },
    },
    Mutation {
        id: 93,
        file: "libs/mmio-range/src/lib.rs",
        find: "        self.remaining -= 1;",
        replace: "        self.remaining += 1;",
        breaks: "an MMIO offset iterator grows instead of reaching exhaustion",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "every_small_repeated_access_matches_an_independent_oracle",
        },
    },
    Mutation {
        id: 94,
        file: "libs/mmio-range/src/lib.rs",
        find: "        (self.remaining, Some(self.remaining))",
        replace: "        (0, None)",
        breaks: "an exact MMIO offset iterator hides its remaining length",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "every_small_repeated_access_matches_an_independent_oracle",
        },
    },
    Mutation {
        id: 95,
        file: "libs/mmio-range/src/lib.rs",
        find: "            Self::InvalidWidth(width) => write!(formatter, \"access width {width} is invalid\"),",
        replace: "            Self::InvalidWidth(_) => formatter.write_str(\"invalid\"),",
        breaks: "an MMIO refusal omits the invalid width and its rule",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "tests::errors_name_the_refused_access",
        },
    },
    Mutation {
        id: 96,
        file: "libs/x86-paging/src/lib.rs",
        find: "        Self(self.0 | other.0)",
        replace: "        Self(self.0 ^ other.0)",
        breaks: "union removes a flag already held by both inputs",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::union_is_idempotent_when_the_sets_overlap",
        },
    },
    Mutation {
        id: 97,
        file: "libs/x86-paging/src/lib.rs",
        find: "            Self::Misaligned(address) => write!(f, \"{address:#x} is not page-aligned\"),",
        replace: "            Self::Misaligned(_) => f.write_str(\"misaligned\"),",
        breaks: "a page-entry refusal omits the address and alignment rule",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::entry_errors_name_the_refused_value_or_rule",
        },
    },
    Mutation {
        id: 98,
        file: "libs/x86-paging/src/lib.rs",
        find: "            Self::ZeroLength => core::fmt::Write::write_str(f, \"mapping length is zero\"),",
        replace: "            Self::ZeroLength => core::fmt::Write::write_str(f, \"zero\"),",
        breaks: "a mapping-range refusal omits the failed length rule",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::mapping_range_errors_name_the_refused_value_or_rule",
        },
    },
    Mutation {
        id: 99,
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "        || !value.bytes().all(|byte| byte.is_ascii_digit())",
        replace: "",
        breaks: "a signed artifact length is accepted as canonical decimal",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::decimals_tokens_and_revisions_enforce_each_canonical_boundary",
        },
    },
    Mutation {
        id: 100,
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "            Self::NonCanonicalLineEnding => \"handoff record does not use canonical LF endings\",",
        replace: "            Self::NonCanonicalLineEnding => \"invalid\",",
        breaks: "a handoff refusal omits the noncanonical line-ending rule",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::errors_name_the_refused_record_property",
        },
    },
    Mutation {
        id: 101,
        file: "libs/frame-space/src/lib.rs",
        find: "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {\n        f.debug_struct(\"FrameSpace\")\n            .field(\"managed\", &self.managed_count)\n            .field(\"free\", &self.free_count)\n            .finish()\n    }",
        replace: "    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {\n        Ok(())\n    }",
        breaks: "the frame allocator diagnostic loses both ledger counts",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::debug_reports_both_ledger_counts",
        },
    },
    Mutation {
        id: 112,
        file: "libs/fat32/src/lib.rs",
        find: "        if sector[510] != 0x55 || sector[511] != 0xAA {",
        replace: "        if sector[510] != 0x55 && sector[511] != 0xAA {",
        breaks: "half of a FAT boot signature is accepted as the whole signature",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_boot_sector_that_is_not_one_is_refused",
        },
    },
    Mutation {
        id: 113,
        file: "libs/fat32/src/lib.rs",
        find: "                + byte / SECTOR_BYTES as u64,",
        replace: "                + 0,",
        breaks: "FAT entries past the first table sector are read from sector zero",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::the_boot_sector_is_read_where_the_layout_puts_its_fields",
        },
    },
    Mutation {
        id: 114,
        file: "libs/fat32/src/lib.rs",
        find: "        wanted\n            .iter()\n            .zip(self.name())\n            .all(|(wanted, actual)| upper(*wanted) == upper(*actual))",
        replace: "        wanted.iter().zip(self.name()).all(|_| true)",
        breaks: "a same-length different FAT name is accepted as a match",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_name_is_matched_without_regard_to_case",
        },
    },
    Mutation {
        id: 115,
        file: "libs/fat32/src/lib.rs",
        find: "        if slot[8 + index] != b' ' {\n            return Slot::Skip;\n        }",
        replace: "        if false && slot[8 + index] != b' ' {\n            return Slot::Skip;\n        }",
        breaks: "a sparse FAT extension loses its only non-space byte",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::an_eight_three_name_is_unpacked",
        },
    },
    Mutation {
        id: 117,
        file: "libs/fat32/src/lib.rs",
        find: "                done += take;",
        replace: "                done = 0;",
        breaks: "a FAT file read makes no forward byte progress",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::file_reads_match_the_concatenated_sector_bytes_at_every_boundary",
        },
    },
    Mutation {
        id: 118,
        file: "libs/fat32/src/lib.rs",
        find: "                if !entry.is_directory() {\n                    return Err(FatError::WrongKind);\n                }\n                cluster = entry.cluster;",
        replace: "                cluster = entry.cluster;",
        breaks: "path resolution descends through a non-directory component",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::path_resolution_descends_only_through_directory_components",
        },
    },
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
    // Record each requested run before the all-or-none preflight finishes.
    // A conflict later in the list then strands all earlier runs.
    Mutation {
        id: 225,
        file: "libs/port-space/src/lib.rs",
        find: "            room[index] = Some(span);
        }

        for span in room.iter().flatten() {
            for port in span.clone() {
                self.reserved[port / PORTS_PER_WORD] |= bit(port);",
        replace: "            for port in span.clone() {
                self.reserved[port / PORTS_PER_WORD] |= bit(port);
            }
            room[index] = Some(span);
        }

        for span in room.iter().flatten() {
            for port in span.clone() {
                self.reserved[port / PORTS_PER_WORD] |= bit(port);",
        breaks: "a refused multi-span reservation records no earlier span",
        reddens: Caught::HostTest {
            package: "port-space",
            name: "tests::a_refused_multi_span_reservation_takes_nothing",
        },
    },
    // Keep the unique handles consumed but leave their bitmap bits set.
    Mutation {
        id: 226,
        file: "libs/port-space/src/lib.rs",
        find: "                self.reserved[port / PORTS_PER_WORD] &= !bit(port);",
        replace: "                self.reserved[port / PORTS_PER_WORD] |= bit(port);",
        breaks: "a transient multi-span release clears every exact port",
        reddens: Caught::HostTest {
            package: "port-space",
            name: "tests::a_multi_span_release_is_exact_and_all_or_none",
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
    // Admit an empty read to the zero-based NLB encoder. In release that
    // underflows to the largest command rather than panicking.
    Mutation {
        id: 243,
        file: "libs/nvme/src/lib.rs",
        find: "        if blocks == 0 {\n            return Err(TransferError::Empty);\n        }",
        replace: "        if false && blocks == 0 {\n            return Err(TransferError::Empty);\n        }",
        breaks: "an empty transfer has no nlb encoding",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::an_empty_transfer_has_no_nlb_encoding",
        },
    },
    // Admit the first count whose zero-based encoding no longer fits the
    // command's sixteen-bit NLB field.
    Mutation {
        id: 244,
        file: "libs/nvme/src/lib.rs",
        find: "        if blocks > MAX_TRANSFER_BLOCKS {\n            return Err(TransferError::TooManyBlocks);\n        }",
        replace: "        if false && blocks > MAX_TRANSFER_BLOCKS {\n            return Err(TransferError::TooManyBlocks);\n        }",
        breaks: "the nlb field has one exact maximum block count",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_nonempty_nlb_count_has_one_exact_boundary",
        },
    },
    // Wrap the LBA end back into the namespace instead of refusing the
    // overflow which would make a controller reach an unrelated block.
    Mutation {
        id: 245,
        file: "libs/nvme/src/lib.rs",
        find: "        let Some(end) = lba.checked_add(blocks as u64) else {\n            return Err(TransferError::OutsideNamespace);\n        };",
        replace: "        let end = lba.wrapping_add(blocks as u64);",
        breaks: "a transfer extent cannot wrap into its namespace",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::a_transfer_never_crosses_or_wraps_its_namespace",
        },
    },
    // Let the device write one byte past the allocation that proves the DMA
    // extent. Exact-size and short buffers differ only at this comparison.
    Mutation {
        id: 246,
        file: "libs/nvme/src/lib.rs",
        find: "        if bytes > buffer_bytes {\n            return Err(TransferError::BufferTooSmall);\n        }",
        replace: "        if false && bytes > buffer_bytes {\n            return Err(TransferError::BufferTooSmall);\n        }",
        breaks: "a transfer fits wholly inside its dma allocation",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::transfer_bytes_must_fit_the_dma_buffer",
        },
    },
    // Treat a zeroed Identify response as a usable namespace. There is no LBA
    // zero when NSZE is zero, however plausible the remaining format looks.
    Mutation {
        id: 248,
        file: "libs/nvme/src/lib.rs",
        find: "        if blocks == 0 {\n            return Err(NamespaceError::Empty);\n        }",
        replace: "        if false && blocks == 0 {\n            return Err(NamespaceError::Empty);\n        }",
        breaks: "a namespace has at least one logical block",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::a_namespace_has_at_least_one_logical_block",
        },
    },
    // Accept the extended format table even though this host never enabled
    // LBAFEE and copied only the sixteen-entry prefix it negotiated.
    Mutation {
        id: 249,
        file: "libs/nvme/src/lib.rs",
        find: "        if last_format >= MAX_LBA_FORMATS {\n            return Err(NamespaceError::TooManyFormats(u16::from(last_format) + 1));\n        }",
        replace: "        if false && last_format >= MAX_LBA_FORMATS {\n            return Err(NamespaceError::TooManyFormats(u16::from(last_format) + 1));\n        }",
        breaks: "only negotiated lba formats are accepted",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_format_selector_stays_inside_the_negotiated_table",
        },
    },
    // Ignore FLBAS's reserved bit, making a malformed selector indistinguishable
    // from a format this host knows how to interpret.
    Mutation {
        id: 250,
        file: "libs/nvme/src/lib.rs",
        find: "        if formatted & 0x80 != 0 {\n            return Err(NamespaceError::ReservedFormatBits(formatted));\n        }",
        replace: "        if false && formatted & 0x80 != 0 {\n            return Err(NamespaceError::ReservedFormatBits(formatted));\n        }",
        breaks: "reserved formatted lba selector bits are refused",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_format_selector_stays_inside_the_negotiated_table",
        },
    },
    // Select a packed table entry beyond zero-based NLBAF. Its bytes are not a
    // supported format even when the fixed Identify buffer happens to hold it.
    Mutation {
        id: 251,
        file: "libs/nvme/src/lib.rs",
        find: "        if selected > last_format {\n            return Err(NamespaceError::FormatOutsideList {\n                selected,\n                formats: last_format + 1,\n            });\n        }",
        replace: "        if false && selected > last_format {\n            return Err(NamespaceError::FormatOutsideList {\n                selected,\n                formats: last_format + 1,\n            });\n        }",
        breaks: "the formatted lba index is inside the reported table",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_format_selector_stays_inside_the_negotiated_table",
        },
    },
    // Interpret reserved LBAF control bits as relative-performance bits. The
    // low two are descriptive; every higher bit is an unknown layout contract.
    Mutation {
        id: 252,
        file: "libs/nvme/src/lib.rs",
        find: "        if control & 0xfc != 0 {\n            return Err(NamespaceError::ReservedLbaFormatBits(control));\n        }",
        replace: "        if false && control & 0xfc != 0 {\n            return Err(NamespaceError::ReservedLbaFormatBits(control));\n        }",
        breaks: "every lba format control bit has an explicit policy",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_lba_format_control_value_has_one_explicit_policy",
        },
    },
    // Drop per-LBA metadata from the transfer length and pointer contract.
    // Even one byte makes the controller's data shape differ from the host's.
    Mutation {
        id: 253,
        file: "libs/nvme/src/lib.rs",
        find: "        if metadata != 0 {\n            return Err(NamespaceError::Metadata(metadata));\n        }",
        replace: "        if false && metadata != 0 {\n            return Err(NamespaceError::Metadata(metadata));\n        }",
        breaks: "selected lba metadata is never silently discarded",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_metadata_and_protection_setting_is_explicit",
        },
    },
    // Confuse support capabilities with active metadata settings. A device may
    // advertise both transfer forms while the selected LBAF uses no metadata.
    Mutation {
        id: 254,
        file: "libs/nvme/src/lib.rs",
        find: "        if metadata != 0 {\n            return Err(NamespaceError::Metadata(metadata));\n        }",
        replace: "        if metadata != 0 || identify[27] != 0 || identify[28] != 0 {\n            return Err(NamespaceError::Metadata(metadata));\n        }",
        breaks: "metadata capabilities do not enable metadata settings",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_metadata_and_protection_setting_is_explicit",
        },
    },
    // Read protected blocks without providing or checking their protection
    // information, turning integrity errors into ordinary data.
    Mutation {
        id: 255,
        file: "libs/nvme/src/lib.rs",
        find: "        if protection != 0 {\n            return Err(NamespaceError::Protection(protection));\n        }",
        replace: "        if false && protection != 0 {\n            return Err(NamespaceError::Protection(protection));\n        }",
        breaks: "enabled protection information is never silently discarded",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_metadata_and_protection_setting_is_explicit",
        },
    },
    // Admit every exponent at or above 512 bytes, restoring the old trust in a
    // byte which may be 32 and which the only storage caller cannot consume.
    Mutation {
        id: 256,
        file: "libs/nvme/src/lib.rs",
        find: "        if block_shift != SUPPORTED_BLOCK_SHIFT {\n            return Err(NamespaceError::BlockSize(block_shift));\n        }",
        replace: "        if block_shift < SUPPORTED_BLOCK_SHIFT {\n            return Err(NamespaceError::BlockSize(block_shift));\n        }",
        breaks: "every lbads byte has one explicit supported policy",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_lbads_value_has_one_explicit_policy",
        },
    },
    // A disk of identical blocks hides a dropped LBA, so the namespace gives
    // every block a distinct signature (D-169).
    Mutation {
        id: 258,
        file: "libs/nvme/src/lib.rs",
        find: "        command.words[10] = lba as u32;
        command.words[11] = (lba >> 32) as u32;",
        replace: "        command.words[10] = 0;
        command.words[11] = 0;",
        breaks: "a read returns the block that was asked for",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::a_command_puts_every_field_where_the_specification_says",
        },
    },
    // The block count sent unadjusted, so one block becomes two. The first
    // block is still exactly right; only the bytes past it say so.
    Mutation {
        id: 259,
        file: "libs/nvme/src/lib.rs",
        find: "        command.words[12] = transfer.zero_based_blocks() as u32;",
        replace: "        command.words[12] = transfer.zero_based_blocks() as u32 + 1;",
        breaks: "every count in this specification is zero-based",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::a_command_puts_every_field_where_the_specification_says",
        },
    },
    // A missing cluster bias reads valid sectors belonging to another file,
    // so the image corpus distinguishes adjacent clusters (D-170).
    Mutation {
        id: 262,
        file: "libs/fat32/src/lib.rs",
        find: "        let index = (cluster - FIRST_DATA_CLUSTER) as u64;",
        replace: "        let index = cluster as u64;",
        breaks: "a cluster number is biased by two",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_clusters_sector_is_biased_by_two",
        },
    },
    // A read bounded by the cluster rather than by the file, which hands back
    // the last cluster's tail: its previous occupant's data.
    Mutation {
        id: 263,
        file: "libs/fat32/src/lib.rs",
        find: "        let wanted = into.len().min((entry.size - offset) as usize);",
        replace: "        let wanted = into.len();",
        breaks: "a read stops at the end of the file",
        reddens: Caught::Check("the kernel reads the file the host wrote"),
    },
    // The deleted marker taken for the end of the directory, which loses every
    // file below it. Only reachable because the corpus deletes one on purpose.
    Mutation {
        id: 264,
        file: "libs/fat32/src/lib.rs",
        find: "    if slot[0] == 0xE5 {
        return Slot::Deleted;
    }",
        replace: "    if slot[0] == 0xE5 {
        return Slot::End;
    }",
        breaks: "a deleted entry is not the end of a directory",
        reddens: Caught::Check("the kernel reads the file the host wrote"),
    },
    // Treating `..` as a name lets a caller ask the resolver to escape its
    // starting handle, contrary to `012` section 5 (D-171).
    Mutation {
        id: 266,
        file: "libs/path-walk/src/lib.rs",
        find: "        if component == b\"..\" {",
        replace: "        if false {",
        breaks: "a resolver cannot be asked to ascend",
        reddens: Caught::Check("no path can name anything above the handle"),
    },
    // Ascending past the start clamped rather than refused, which silently
    // answers a different question than the caller asked.
    Mutation {
        id: 267,
        file: "libs/path-walk/src/lib.rs",
        find: "            depth = depth.checked_sub(1).ok_or(PathError::Escapes)?;",
        replace: "            depth = depth.saturating_sub(1);",
        breaks: "an escape is refused rather than clamped",
        reddens: Caught::Check("no path can name anything above the handle"),
    },
    // A component that is not a directory walked through, so a file's bytes
    // are read as directory entries — with the caller choosing the bytes.
    Mutation {
        id: 268,
        file: "libs/fat32/src/lib.rs",
        find: "                if !entry.is_directory() {
                    return Err(FatError::WrongKind);
                }",
        replace: "                if false {
                    return Err(FatError::WrongKind);
                }",
        breaks: "a file cannot be walked through",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::path_resolution_descends_only_through_directory_components",
        },
    },
    // A run offered from a region that is described but is not memory, which
    // is the hole the four probes fell into on VMware (D-177).
    Mutation {
        id: 285,
        file: "libs/memory-map/src/lib.rs",
        find: "            stored[len] = region;\n            len += 1;",
        replace: "            if region.kind.is_direct_mapped() {\n                stored[len] = region;\n            }\n            len += 1;",
        breaks: "a probed run is memory, not merely described",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::every_accepted_region_informs_memory_policy",
        },
    },
    // Distinct keys keep the translated and untranslated rounds independent.
    Mutation {
        id: 288,
        file: "libs/ps2-keys/src/set1.rs",
        find: "        0x30 => Key::B,",
        replace: "        0x30 => Key::Pause,",
        breaks: "set 1's numbering is the specification's",
        reddens: Caught::HostTest {
            package: "ps2-keys",
            name: "set1::tests::the_letter_rows_are_contiguous_runs",
        },
    },
    Mutation {
        id: 291,
        file: "libs/ps2-keys/src/arriving.rs",
        find: "const BREAK: u8 = 0xF0;",
        replace: "const BREAK: u8 = 0xF1;",
        breaks: "set 2's break prefix is the byte set 2 actually sends",
        reddens: Caught::HostTest {
            package: "ps2-keys",
            name: "arriving::tests::one_keystroke_from_either_machine_decides_it",
        },
    },
    Mutation {
        id: 292,
        file: "libs/ps2-keys/src/arriving.rs",
        find: "const SET_2_HIGHEST_MAKE: u8 = 0x83;",
        replace: "const SET_2_HIGHEST_MAKE: u8 = 0x7F;",
        breaks: "the probe's bounds are the two tables', not numbers beside them",
        reddens: Caught::HostTest {
            package: "ps2-keys",
            name: "arriving::tests::the_two_tables_still_bound_this_probe",
        },
    },
    Mutation {
        id: 293,
        file: "libs/ps2-keys/src/arriving.rs",
        find: "const AMBIGUOUS: [u8; 2] = [0xAA, 0xAB];",
        replace: "const AMBIGUOUS: [u8; 2] = [0xAA, 0xAA];",
        breaks: "a reply in the stream cannot answer what the stream is asked",
        reddens: Caught::HostTest {
            package: "ps2-keys",
            name: "arriving::tests::an_identity_reply_in_the_stream_decides_nothing",
        },
    },
    Mutation {
        id: 296,
        file: "libs/memory-map/src/lib.rs",
        find: "                region.len != 0\n                    && region.base < end\n                    && start < region.end()\n                    && !allowed(region.kind)",
        replace: "                region.len != 0\n                    && region.base <= start\n                    && start < region.end()\n                    && !allowed(region.kind)",
        breaks: "a forbidden claim nested inside an allowed span refuses the whole range",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "coverage_matches_every_byte_claim_in_a_bounded_space",
        },
    },
    Mutation {
        id: 297,
        file: "libs/memory-map/src/lib.rs",
        find: "                if first.is_none_or(|known| base < known) && self.covers_memory(base, end) {",
        replace: "                if first.is_none_or(|known| base < known) && self.covers(base, end) {",
        breaks: "a named memory run is one the coverage contract also calls memory",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::a_named_memory_run_is_memory_by_the_other_contract",
        },
    },
    Mutation {
        id: 328,
        file: "libs/mmio-range/src/lib.rs",
        find: "    for _ in 0..=occupied.len() {",
        replace: "    for _ in 0..occupied.len() {",
        breaks: "first-fit gives up before checking the gap after every occupied extent",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "tests::first_fit_reuses_an_interior_gap_in_any_active_order",
        },
    },
    Mutation {
        id: 329,
        file: "libs/mmio-range/src/lib.rs",
        find: "            return Some(cursor);",
        replace: "            return cursor.checked_add(align);",
        breaks: "first-fit skips the earliest aligned byte in an available gap",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "every_small_first_fit_matches_a_bytewise_oracle",
        },
    },
    Mutation {
        id: 330,
        file: "libs/mmio-range/src/lib.rs",
        find: "            base: base & !(align - 1),",
        replace: "            base,",
        breaks: "a covering MMIO range begins after the unaligned bytes it must include",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "tests::a_covering_range_refuses_rounding_past_the_address_space",
        },
    },
    Mutation {
        id: 331,
        file: "libs/mmio-range/src/lib.rs",
        find: "    pub const fn new(base: u64, len: u64) -> Option<Self> {\n        if len == 0 {",
        replace: "    pub const fn new(base: u64, len: u64) -> Option<Self> {\n        if false {",
        breaks: "an empty MMIO address range is admitted as a usable extent",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "tests::address_ranges_refuse_empty_and_wrapping_bounds",
        },
    },
    Mutation {
        id: 334,
        file: "libs/timebase/src/lib.rs",
        find: "            Some(now) => Self::Deadline(now.saturating_add(timeout_ms.saturating_mul(1_000_000))),",
        replace: "            Some(_) => Self::Attempts(fallback_attempts),",
        breaks: "a calibrated poll is bounded by real time rather than iteration rate",
        reddens: Caught::HostTest {
            package: "timebase",
            name: "tests::a_monotonic_poll_limit_expires_by_time_not_iteration_rate",
        },
    },
    Mutation {
        id: 335,
        file: "libs/timebase/src/lib.rs",
        find: "            Self::Attempts(remaining) if *remaining > 0 => {",
        replace: "            Self::Attempts(remaining) if *remaining > 1 => {",
        breaks: "a clockless poll spends exactly its declared fallback attempts",
        reddens: Caught::HostTest {
            package: "timebase",
            name: "tests::a_clockless_poll_limit_permits_its_exact_fallback_count",
        },
    },
    Mutation {
        id: 336,
        file: "libs/timebase/src/lib.rs",
        find: "            Some(now) => Self::Deadline(now.saturating_add(timeout_ms.saturating_mul(1_000_000))),",
        replace: "            Some(now) => Self::Deadline(now.wrapping_add(timeout_ms.saturating_mul(1_000_000))),",
        breaks: "a poll deadline saturates instead of wrapping into the past",
        reddens: Caught::HostTest {
            package: "timebase",
            name: "tests::a_deadline_saturates_instead_of_wrapping_into_the_past",
        },
    },
    Mutation {
        id: 348,
        file: "libs/handle-space/src/lib.rs",
        find: "    identity.checked_add(1)",
        replace: "    Some(identity.wrapping_add(1))",
        breaks: "table identity arithmetic wraps its exhaustion sentinel to zero",
        reddens: Caught::HostTest {
            package: "handle-space",
            name: "tests::table_identity_exhaustion_never_wraps",
        },
    },
    Mutation {
        id: 351,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    model.io_apic_count += 1;",
        replace: "                    model.io_apic_count = 1;",
        breaks: "the MADT inventory discards every I/O APIC after the first",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_bounded_interrupt_inventory_keeps_every_controller",
        },
    },
    Mutation {
        id: 352,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    let Some(slot) = model.io_apics.get_mut(model.io_apic_count) else {\n                        return Err(InterruptError::TooManyIoApics { limit: IO_APICS });\n                    };",
        replace: "                    let Some(slot) = model.io_apics.get_mut(model.io_apic_count) else {\n                        break;\n                    };",
        breaks: "an over-capacity MADT is silently truncated into a usable inventory",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::an_interrupt_inventory_refuses_overflow_instead_of_truncating",
        },
    },
    Mutation {
        id: 353,
        file: "libs/x86-apic/src/io.rs",
        find: "        let Some(distance) = entries.checked_sub(1) else {\n            return None;\n        };",
        replace: "        let distance = entries.saturating_sub(1);",
        breaks: "an I/O APIC reporting zero entries claims its base GSI",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "io::tests::a_range_is_non_empty_and_stays_inside_the_gsi_namespace",
        },
    },
    Mutation {
        id: 354,
        file: "libs/x86-apic/src/io.rs",
        find: "    Ok(ranges.iter().position(|range| range.contains(gsi)))",
        replace: "    Ok(ranges\n        .first()\n        .filter(|range| range.contains(gsi))\n        .map(|_| 0))",
        breaks: "GSI ownership search stops at the first I/O APIC",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "io::tests::a_gsi_selects_its_later_controller_across_a_gap",
        },
    },
    Mutation {
        id: 355,
        file: "libs/x86-apic/src/io.rs",
        find: "            if ranges[first].overlaps(ranges[second]) {",
        replace: "            if ranges[first].overlaps(ranges[second]) && first == second {",
        breaks: "two I/O APICs may ambiguously own the same GSI",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "io::tests::any_overlap_refuses_the_complete_inventory",
        },
    },
    Mutation {
        id: 356,
        file: "libs/fat32/src/lib.rs",
        find: "#![cfg_attr(\n    not(test),\n    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)\n)]",
        replace: "",
        breaks: "a hostile-input parser regains expect, unwrap and panic shortcuts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "boundary::tests::target_boundaries_keep_panicking_shortcuts_out",
        },
    },
    Mutation {
        id: 358,
        file: "libs/fat32/src/lib.rs",
        find: "        if first_data_sector >= total_sectors as u64 {",
        replace: "        if false && first_data_sector >= total_sectors as u64 {",
        breaks: "a FAT table extent at or beyond the volume is accepted",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::impossible_volume_geometry_is_refused_before_arithmetic",
        },
    },
    Mutation {
        id: 359,
        file: "libs/fat32/src/lib.rs",
        find: "        if (root_cluster - FIRST_DATA_CLUSTER) as u64 >= data_clusters {",
        replace: "        if false && (root_cluster - FIRST_DATA_CLUSTER) as u64 >= data_clusters {",
        breaks: "a FAT root directory may begin outside the complete data clusters",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::the_root_cluster_must_lie_inside_the_data_area",
        },
    },
    Mutation {
        id: 360,
        file: "libs/fat32/src/lib.rs",
        find: "        if data_clusters + FIRST_DATA_CLUSTER as u64 > fat_entries {",
        replace: "        if false && data_clusters + FIRST_DATA_CLUSTER as u64 > fat_entries {",
        breaks: "a FAT too short to index every advertised data cluster is accepted",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::the_fat_indexes_every_advertised_data_cluster",
        },
    },
    Mutation {
        id: 361,
        file: "libs/fat32/src/lib.rs",
        find: "        if data_clusters < MIN_FAT32_CLUSTERS || data_clusters > MAX_FAT32_CLUSTERS {",
        replace: "        if false || data_clusters > MAX_FAT32_CLUSTERS {",
        breaks: "a FAT12 or FAT16 cluster count is accepted as FAT32",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_fat32_volume_has_at_least_the_fat32_cluster_count",
        },
    },
    Mutation {
        id: 362,
        file: "libs/fat32/src/lib.rs",
        find: "        if data_clusters < MIN_FAT32_CLUSTERS || data_clusters > MAX_FAT32_CLUSTERS {",
        replace: "        if data_clusters < MIN_FAT32_CLUSTERS || false {",
        breaks: "a FAT32 data domain reaches reserved cluster numbers",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_fat32_volume_stops_before_reserved_cluster_numbers",
        },
    },
    Mutation {
        id: 363,
        file: "libs/fat32/src/lib.rs",
        find: "            || (extended_flags & MIRRORING_DISABLED == 0 && selector != 0)",
        replace: "            || false",
        breaks: "a mirrored FAT32 volume accepts a meaningless inactive-table selector",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::extended_flags_reject_reserved_bits_and_meaningless_selectors",
        },
    },
    Mutation {
        id: 364,
        file: "libs/fat32/src/lib.rs",
        find: "            || active_fat >= fats",
        replace: "            || false",
        breaks: "an active FAT selector may name a table the volume does not have",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::an_active_fat_must_name_an_existing_copy",
        },
    },
    Mutation {
        id: 365,
        file: "libs/fat32/src/lib.rs",
        find: "                + self.active_fat as u64 * self.sectors_per_fat as u64",
        replace: "                + self.active_fat as u64 * 0",
        breaks: "FAT reads ignore the selected active table",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::extended_flags_choose_the_fat_that_is_read",
        },
    },
    Mutation {
        id: 366,
        file: "libs/fat32/src/lib.rs",
        find: "        if let Next::Cluster(cluster) = next\n            && !self.bpb.holds(cluster)\n        {\n            return Err(FatError::BadCluster(cluster));\n        }",
        replace: "",
        breaks: "a FAT chain link may name a marker or a cluster outside this volume",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_chain_link_must_name_a_cluster_in_this_volume",
        },
    },
    Mutation {
        id: 367,
        file: "libs/fat32/src/lib.rs",
        find: "        let skip_clusters = offset / cluster_bytes;",
        replace: "        let skip_clusters = offset;",
        breaks: "a FAT byte offset is mistaken for a cluster count",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_maximum_file_offset_does_not_overflow_chain_progress",
        },
    },
    Mutation {
        id: 368,
        file: "libs/fat32/src/lib.rs",
        find: "    const fn chain_limit(&self) -> u32 {\n        self.bpb.clusters()\n    }",
        replace: "    const fn chain_limit(&self) -> u32 {\n        1 << 24\n    }",
        breaks: "a FAT chain walk is bounded by the volume that could hold it",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_file_skip_past_the_volume_is_refused_before_walking",
        },
    },
    Mutation {
        id: 370,
        file: "libs/fat32/src/lib.rs",
        find: "                Next::Cluster(next) => {\n                    let Some(visited) = cursor.visited.checked_add(1) else {\n                        return Err(FatError::Unbounded);\n                    };\n                    if visited >= self.bpb.directory_cluster_limit() {\n                        return Err(FatError::Unbounded);\n                    }\n                    // A tortoise/hare check catches a cycle in the length of\n                    // the cycle, before the entry cap is reached (D-352).\n                    let hare = self.advance_cycle_hare(cursor.hare)?;\n                    if hare == Some(next) {\n                        return Err(FatError::Unbounded);\n                    }\n                    cursor.cluster = next;\n                    cursor.sector = 0;\n                    cursor.visited = visited;\n                    cursor.hare = hare;\n                }\n                Next::End => {\n                    cursor.done = true;\n                    return Ok(DirectoryChunk::EMPTY);\n                }",
        replace: "                Next::Cluster(_) | Next::End => {\n                    cursor.done = true;\n                    return Ok(DirectoryChunk::EMPTY);\n                }",
        breaks: "a FAT directory walk silently stops after its first cluster",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::foreign::both_readers_agree_on_the_adversarial_foreign_layout",
        },
    },
    Mutation {
        id: 388,
        file: "libs/panic-record/src/lib.rs",
        find: "                '\\n' => Some(\"\\\\n\"),",
        replace: "                '\\n' => None,",
        breaks: "a panic body can forge its terminal record with a physical newline",
        reddens: Caught::HostTest {
            package: "panic-record",
            name: "tests::a_hostile_body_cannot_forge_the_terminal_record",
        },
    },
    Mutation {
        id: 389,
        file: "libs/panic-record/src/lib.rs",
        find: "                '\\r' => Some(\"\\\\r\"),",
        replace: "                '\\r' => None,",
        breaks: "a panic body can retain a physical carriage return",
        reddens: Caught::HostTest {
            package: "panic-record",
            name: "tests::every_ascii_character_uses_the_canonical_escape_encoding",
        },
    },
    Mutation {
        id: 390,
        file: "libs/panic-record/src/lib.rs",
        find: "                '\\\\' => Some(\"\\\\\\\\\"),",
        replace: "                '\\\\' => None,",
        breaks: "a literal escape sequence is indistinguishable from a physical panic line break",
        reddens: Caught::HostTest {
            package: "panic-record",
            name: "tests::the_escape_encoding_is_injective_over_ascii_pairs",
        },
    },
    Mutation {
        id: 391,
        file: "libs/panic-record/src/lib.rs",
        find: "            core::fmt::Write::write_str(&mut *self.writer, escaped)?;",
        replace: "            let _ = core::fmt::Write::write_str(&mut *self.writer, escaped);",
        breaks: "an escaped panic segment can fail without withholding completion",
        reddens: Caught::HostTest {
            package: "panic-record",
            name: "tests::writer_errors_propagate_from_plain_and_escaped_segments",
        },
    },
    Mutation {
        id: 392,
        file: "libs/panic-record/src/lib.rs",
        find: "                core::fmt::Write::write_str(&mut *self.writer, &text[start..index])?;",
        replace: "                let _ = core::fmt::Write::write_str(&mut *self.writer, &text[start..index]);",
        breaks: "a refused plain segment before an escape is reported as written",
        reddens: Caught::HostTest {
            package: "panic-record",
            name: "tests::writer_errors_propagate_from_plain_and_escaped_segments",
        },
    },
    Mutation {
        id: 405,
        file: "libs/panic-record/src/lib.rs",
        find: "    writeln!(writer, \"[error] {COMPLETE_MARKER}\")",
        replace: "    Ok(())",
        breaks: "the kernel omits the terminal panic record",
        reddens: Caught::HostTest {
            package: "panic-record",
            name: "tests::completion_requires_every_prior_character_write",
        },
    },
    Mutation {
        id: 406,
        file: "libs/panic-record/src/lib.rs",
        find: "        write!(record, \"{OPEN_PREFIX}{thread}': {body}\")?;",
        replace: "        let _ = write!(record, \"{OPEN_PREFIX}{thread}': {body}\");",
        breaks: "the kernel terminates a panic record whose body write failed",
        reddens: Caught::HostTest {
            package: "panic-record",
            name: "tests::completion_requires_every_prior_character_write",
        },
    },
    Mutation {
        id: 408,
        file: "libs/panic-record/src/lib.rs",
        find: "    writer.write_char('\\n')?;",
        replace: "    let _ = writer.write_char('\\n');",
        breaks: "the kernel emits completion after its body separator was refused",
        reddens: Caught::HostTest {
            package: "panic-record",
            name: "tests::completion_requires_every_prior_character_write",
        },
    },
    Mutation {
        id: 415,
        file: "libs/panic-record/src/lib.rs",
        find: "#![cfg_attr(\n    not(test),\n    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)\n)]",
        replace: "",
        breaks: "arbitrary panic text regains production panic shortcuts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "boundary::tests::target_boundaries_keep_panicking_shortcuts_out",
        },
    },
    Mutation {
        id: 419,
        file: "libs/eevdf/src/lib.rs",
        find: "        if self.slice_is_spent() {\n            return 0;\n        }",
        replace: "        if false && self.slice_is_spent() {\n            return 0;\n        }",
        breaks: "a thread past its deadline wraps into an enormous remaining slice",
        reddens: Caught::HostTest {
            package: "eevdf",
            name: "tests::a_thread_past_its_deadline_has_no_slice_left",
        },
    },
    Mutation {
        id: 420,
        file: "libs/elf64/src/lib.rs",
        find: "        match self.offset.checked_add(self.file_size) {\n            Some(end) => end <= bytes as u64,\n            None => false,\n        }",
        replace: "        self.offset.wrapping_add(self.file_size) <= bytes as u64",
        breaks: "a segment extent wraps around into the elf image",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::a_loader_can_ask_whether_the_contents_fit",
        },
    },
    Mutation {
        id: 421,
        file: "libs/frame-space/src/lib.rs",
        find: "        for frame in first..end {\n            self.check_releasable(frame)?;\n        }",
        replace: "        self.check_releasable(first)?;",
        breaks: "a multi-frame free mutates before its complete run is valid",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::freeing_a_run_is_all_or_nothing",
        },
    },
    Mutation {
        id: 422,
        file: "libs/mmio-range/src/lib.rs",
        find: "        if end > region_len {",
        replace: "        if end > region_len.saturating_add(1) {",
        breaks: "an mmio access may end one byte beyond its claimed region",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "tests::a_span_one_byte_past_the_region_is_refused",
        },
    },
    Mutation {
        id: 423,
        file: "libs/path-walk/src/lib.rs",
        find: "            depth = depth.checked_sub(1).ok_or(PathError::Escapes)?;",
        replace: "            depth = depth.saturating_sub(1);",
        breaks: "ascending above a directory handle is clamped to its root",
        reddens: Caught::HostTest {
            package: "path-walk",
            name: "tests::ascending_past_the_start_is_refused_not_clamped",
        },
    },
    Mutation {
        id: 424,
        file: "libs/pixel/src/lib.rs",
        find: "        let Some(span) = pitch.checked_mul(height) else {",
        replace: "        let Some(span) = row_bytes.checked_mul(height) else {",
        breaks: "a padded framebuffer mapping omits every row's padding",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::a_padded_framebuffer_spans_its_pitch",
        },
    },
    Mutation {
        id: 425,
        file: "libs/ps2-keys/src/decode.rs",
        find: "            Set::One => protocol::is_reply(byte) && byte != protocol::SELF_TEST_PASSED,",
        replace: "            Set::One => protocol::is_reply(byte),",
        breaks: "set one swallows a left-shift release as a controller reply",
        reddens: Caught::HostTest {
            package: "ps2-keys",
            name: "decode::tests::set_1_reads_aa_as_a_shift_release_not_a_self_test_result",
        },
    },
    Mutation {
        id: 426,
        file: "libs/ring-proto/src/lib.rs",
        find: "        if self.reserved + self.completion.occupied() >= self.completion.capacity().entries() {",
        replace: "        if self.reserved >= self.completion.capacity().entries() {",
        breaks: "ring admission ignores completions already occupying their queue",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "tests::the_reservation_invariant_holds_under_interleaving",
        },
    },
    Mutation {
        id: 427,
        file: "libs/slab-layout/src/lib.rs",
        find: "        Ok(None) if size == 0 => Ok(Placement::Frames {\n            count: 1,\n            align: align.div_ceil(FRAME_SIZE),\n        }),",
        replace: "        Ok(None) if false && size == 0 => Ok(Placement::Frames {\n            count: 1,\n            align: align.div_ceil(FRAME_SIZE),\n        }),",
        breaks: "an over-aligned zero-size allocation asks for zero frames",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "tests::a_zero_sized_request_too_aligned_for_a_class_still_gets_a_frame",
        },
    },
    Mutation {
        id: 428,
        file: "libs/stack-layout/src/lib.rs",
        find: "            stack_base: guard_base + self.guard_len,",
        replace: "            stack_base: guard_base,",
        breaks: "a stack begins on the page reserved as its guard",
        reddens: Caught::HostTest {
            package: "stack-layout",
            name: "tests::every_stack_has_its_guard_immediately_below",
        },
    },
    Mutation {
        id: 429,
        file: "libs/timebase/src/lib.rs",
        find: "        let carried = advanced - (whole << self.shift);",
        replace: "        let carried = 0;",
        breaks: "each clock update discards its fractional nanosecond carry",
        reddens: Caught::HostTest {
            package: "timebase",
            name: "published_tests::a_thousand_updates_do_not_drift_from_one_conversion",
        },
    },
    Mutation {
        id: 430,
        file: "libs/trap-cause/src/lib.rs",
        find: "        self.0 & (1 << 1) != 0",
        replace: "        self.0 & (1 << 2) != 0",
        breaks: "a page fault reports user access as a write",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::every_page_fault_bit_is_read_on_its_own",
        },
    },
    Mutation {
        id: 431,
        file: "libs/x86-cpuid/src/xstate.rs",
        find: "pub const fn has_permitted_components(available: u64) -> bool {\n    available & PERMITTED == PERMITTED\n}",
        replace: "pub const fn has_permitted_components(available: u64) -> bool {\n    available & PERMITTED != 0\n}",
        breaks: "a partial x87, SSE or AVX mask can satisfy the fixed state policy",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "xstate::tests::every_permitted_component_must_be_available",
        },
    },
    Mutation {
        id: 432,
        file: "libs/x86-cpuid/src/xstate.rs",
        find: "    if !component.is_present() || offset < HEADER_BYTES as u32 {",
        replace: "    if !component.is_present() || offset > HEADER_BYTES as u32 {",
        breaks: "a standard XSAVE component can overlap the legacy area",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "xstate::tests::a_standard_component_extent_uses_the_reported_offset",
        },
    },
    Mutation {
        id: 433,
        file: "libs/x86-cpuid/src/xstate.rs",
        find: "        let mut complete = mask & AVX == 0 || mask & SSE != 0;",
        replace: "        let mut complete = true;",
        breaks: "AVX is enabled without its SSE dependency",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "xstate::tests::avx_without_sse_is_not_consistent",
        },
    },
    Mutation {
        id: 434,
        file: "libs/x86-cpuid/src/xstate.rs",
        find: "    mask & AVX == 0 || basic_ecx & AVX_SUPPORTED != 0",
        replace: "    mask & AVX == 0 || basic_ecx & AVX_SUPPORTED == 0",
        breaks: "AVX state is enabled when CPUID does not advertise AVX instructions",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "xstate::tests::avx_state_requires_avx_instruction_support",
        },
    },
    Mutation {
        id: 435,
        file: "libs/x86-cpuid/src/xstate.rs",
        find: "            if !component.is_present() {\n                complete = false;\n                break;\n            }",
        replace: "            if !component.is_present() {\n                continue;\n            }",
        breaks: "an enabled extended-state component has no CPUID sub-leaf",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "xstate::tests::every_enabled_extended_component_must_be_present",
        },
    },
    Mutation {
        id: 436,
        file: "libs/x86-cpuid/src/xstate.rs",
        find: "            let Ok(narrowed) = u32::try_from(cursor) else {\n                complete = false;\n                break;\n            };",
        replace: "            let narrowed = cursor as u32;",
        breaks: "a compacted xstate offset truncates through the u32 boundary",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "xstate::tests::a_layout_that_overflows_a_u32_offset_is_not_consistent",
        },
    },
    Mutation {
        id: 437,
        file: "libs/x86-paging/src/lib.rs",
        find: "        Self(self.0 & other.0)",
        replace: "        Self(self.0 | other.0)",
        breaks: "a page walk unions permissions that every level must grant",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::intersection_keeps_only_what_both_hold",
        },
    },
    Mutation {
        id: 438,
        file: "libs/x86-pit/src/calibrate.rs",
        find: "        Some(product) => product.checked_add(denominator / 2),",
        replace: "        Some(product) => product.checked_add(denominator & 0),",
        breaks: "pit calibration floors instead of rounding to nearest",
        reddens: Caught::HostTest {
            package: "x86-pit",
            name: "calibrate::tests::rounding_is_to_nearest_not_floor",
        },
    },
    Mutation {
        id: 439,
        file: "libs/x86-tables/src/idt.rs",
        find: "        gate.attributes |= 0b11 << 5;",
        replace: "        gate.attributes |= 0b11 << 4;",
        breaks: "a user-callable interrupt gate writes the wrong privilege bits",
        reddens: Caught::HostTest {
            package: "x86-tables",
            name: "idt::tests::a_user_callable_gate_differs_only_in_privilege",
        },
    },
    Mutation {
        id: 444,
        file: "libs/elf64/src/lib.rs",
        find: "            offset: read_u64(header, 40)?,",
        replace: "            offset: read_u64(header, 32)?,",
        breaks: "the section table offset is read from the program table field",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::the_section_table_fields_are_where_the_file_header_puts_them",
        },
    },
    Mutation {
        id: 448,
        file: "libs/mmio-range/src/lib.rs",
        find: "        if count != 0 && !offset.is_multiple_of(width) {",
        replace: "        if !offset.is_multiple_of(width) {",
        breaks: "an empty mmio span acquires an alignment requirement despite accessing nothing",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "every_small_repeated_access_matches_an_independent_oracle",
        },
    },
    Mutation {
        id: 449,
        file: "libs/mmio-range/src/lib.rs",
        find: "        self.next += self.width;",
        replace: "        self.next += 0;",
        breaks: "a checked MMIO span repeats its first offset instead of covering the range",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "every_small_repeated_access_matches_an_independent_oracle",
        },
    },
    Mutation {
        id: 450,
        file: "libs/stack-layout/src/lib.rs",
        find: "    left < right_end && right < left_end",
        replace: "    left <= right_end && right < left_end",
        breaks: "a range ending beside a stack is mistaken for an overlap",
        reddens: Caught::HostTest {
            package: "stack-layout",
            name: "half_open_ranges_distinguish_crossing_adjacent_and_one_past",
        },
    },
    Mutation {
        id: 497,
        file: "libs/memory-map/src/lib.rs",
        find: "            if len == MAX_REGIONS {
                return Err(MemoryMapError::TooMany {
                    observed: MAX_REGIONS + 1,
                    capacity: MAX_REGIONS,
                });
            }",
        replace: "            if len == MAX_REGIONS {
                break;
            }",
        breaks: "an oversized boot map cannot become an accepted prefix",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::the_region_limit_is_all_or_nothing",
        },
    },
    Mutation {
        id: 498,
        file: "libs/x86-cpuid/src/lib.rs",
        find: "                let product = (crystal_hz as u64).checked_mul(numerator as u64);",
        replace: "                let product = (crystal_hz as u64).checked_mul(denominator as u64);",
        breaks: "the CPUID TSC ratio cannot exchange its numerator and denominator",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::frequency_leaves_keep_exact_and_nominal_evidence_distinct",
        },
    },
    Mutation {
        id: 499,
        file: "libs/x86-pit/src/calibrate.rs",
        find: "    if observed_hz.abs_diff(expected_tsc_hz) <= expected_tsc_hz / AGREEMENT_DIVISOR {",
        replace: "    if observed_hz.abs_diff(observed_hz) <= expected_tsc_hz / AGREEMENT_DIVISOR {",
        breaks: "a stopped TSC cannot agree with an advancing PIT",
        reddens: Caught::HostTest {
            package: "x86-pit",
            name: "calibrate::tests::the_watchdog_distinguishes_disagreement_from_delay",
        },
    },
    Mutation {
        id: 500,
        file: "libs/x86-pit/src/calibrate.rs",
        find: "    if apic_remaining != 0 || expected_pit_ticks == 0 || expected_tsc_hz == 0 {",
        replace: "    if false && apic_remaining != 0 || expected_pit_ticks == 0 || expected_tsc_hz == 0 {",
        breaks: "an older pending APIC interrupt cannot judge a newly armed timer",
        reddens: Caught::HostTest {
            package: "x86-pit",
            name: "calibrate::tests::the_watchdog_distinguishes_disagreement_from_delay",
        },
    },
    Mutation {
        id: 504,
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "pub const FORMAT: &str = \"wizard-wizzy-handoff-v1\";",
        replace: "pub const FORMAT: &str = \"wizard-wizzy-handoff-v2\";",
        breaks: "the pinned handoff example uses the one supported envelope version",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::a_canonical_record_is_accepted",
        },
    },
    Mutation {
        id: 505,
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    if core::iter::Iterator::next(&mut lines) != Some(FORMAT) {",
        replace: "    if core::iter::Iterator::next(&mut lines).is_none() {",
        breaks: "an unknown handoff envelope version cannot be parsed as version one",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::an_unknown_envelope_version_is_refused",
        },
    },
    Mutation {
        id: 506,
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    extra.is_none()",
        replace: "    extra.is_none() || true",
        breaks: "an unknown handoff field cannot be silently ignored",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::unknown_or_noncanonical_fields_are_refused",
        },
    },
    Mutation {
        id: 507,
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    first.is_ascii_lowercase()",
        replace: "    true",
        breaks: "handoff values cannot use ambiguous encodings or ambient provenance",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::decimals_tokens_and_revisions_enforce_each_canonical_boundary",
        },
    },
    Mutation {
        id: 508,
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    record.target_id == TARGET_ID",
        replace: "    true || record.target_id == TARGET_ID",
        breaks: "every handoff target and ABI version must match exactly",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::every_version_must_match_exactly",
        },
    },
    Mutation {
        id: 509,
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    record.artifact_bytes == artifact.bytes && record.artifact_sha256 == artifact.sha256",
        replace: "    record.artifact_bytes == artifact.bytes || record.artifact_sha256 == artifact.sha256",
        breaks: "artifact length and digest must both match the consumed bytes",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::artifact_length_and_digest_identify_the_same_bytes",
        },
    },
    Mutation {
        id: 523,
        file: "libs/ring-proto/src/lib.rs",
        find: "                self.reserved += 1;",
        replace: "                self.reserved += 0;",
        breaks: "an admitted operation has no completion reservation",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "scheduled_calls_preserve_the_production_reservation",
        },
    },
    // The property tier's own falsifiability (D-242): the defect must
    // survive every generated case, not one a unit test names.
    Mutation {
        id: 549,
        file: "libs/path-walk/src/lib.rs",
        find: "        if component == b\".\" {",
        replace: "        if false && component == b\".\" {",
        breaks: "a bare dot is a no-op rather than a component the walker descends into",
        reddens: Caught::HostTest {
            package: "path-walk",
            name: "no_split_output_can_name_an_ancestor",
        },
    },
    Mutation {
        id: 550,
        file: "libs/stack-layout/src/lib.rs",
        find: "    if left_len == 0 || right_len == 0 {",
        replace: "    if left_len == 0 {",
        breaks: "an empty range on either side touches nothing, not merely the left",
        reddens: Caught::HostTest {
            package: "stack-layout",
            name: "an_empty_range_touches_nothing",
        },
    },
    Mutation {
        id: 551,
        file: "libs/frame-space/src/lib.rs",
        find: "        if !self.is_managed(frame) {\n            return Err(FreeError::NotManaged(frame));\n        }",
        replace: "        if !self.is_managed(frame) && frame >= self.managed_count {\n            return Err(FreeError::NotManaged(frame));\n        }",
        breaks: "an in-range hole is authorized by the managed-frame cardinality",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "release_authority_distinguishes_managed_in_range_unmanaged_and_out_of_range",
        },
    },
    Mutation {
        id: 552,
        file: "libs/nvme/src/lib.rs",
        find: "            && address.checked_add(len).is_some()",
        replace: "            && true",
        breaks: "a command buffer's contiguous physical extent can wrap the address space",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::identify_binds_one_complete_aligned_page",
        },
    },
    Mutation {
        id: 554,
        file: "libs/timebase/src/lib.rs",
        find: "        if multiplier > MAX_MULTIPLIER {\n            return None;\n        }",
        replace: "",
        breaks: "raw timebase construction publishes a multiplier wider than its wire field",
        reddens: Caught::HostTest {
            package: "timebase",
            name: "tests::raw_parts_refuse_a_multiplier_that_cannot_be_published",
        },
    },
    Mutation {
        id: 555,
        file: "libs/timebase/src/lib.rs",
        find: "        if shift > MAX_SHIFT {\n            return None;\n        }",
        replace: "",
        breaks: "raw timebase construction accepts an unevaluable shift",
        reddens: Caught::HostTest {
            package: "timebase",
            name: "tests::raw_parts_refuse_a_shift_that_cannot_be_evaluated",
        },
    },
    Mutation {
        id: 556,
        file: "libs/path-walk/src/lib.rs",
        find: "#[cfg(test)]\nmod tests",
        replace: "/// Whether `path` begins as a relative path.\npub const fn is_relative(path: &[u8]) -> bool {\n    !path.is_empty() && path[0] != SEPARATOR\n}\n\n#[cfg(test)]\nmod tests",
        breaks: "a weaker duplicate path predicate re-enters the public compiler surface",
        reddens: Caught::ApiAudit("fn is_relative"),
    },
    Mutation {
        id: 557,
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    if manifest.contains('\\r') || !manifest.ends_with('\\n') {",
        replace: "    if manifest.contains('\\r') {",
        breaks: "a Wizzy handoff without its canonical terminal line feed is accepted",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::a_canonical_record_has_exactly_one_terminal_lf",
        },
    },
    Mutation {
        id: 558,
        file: "libs/x86-paging/src/lib.rs",
        find: "            (Self::OWNED, \"owned\"),",
        replace: "",
        breaks: "page-table diagnostics omit software ownership authority",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::debug_says_what_the_entry_is",
        },
    },
    Mutation {
        id: 559,
        file: "libs/ring-proto/src/lib.rs",
        find: "                self.reserved -= 1;",
        replace: "                self.reserved -= 0;",
        breaks: "a reported operation keeps its completion reservation",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "scheduled_calls_preserve_the_production_reservation",
        },
    },
    Mutation {
        id: 560,
        file: "libs/trap-cause/src/lib.rs",
        find: "}\n\n/// An architectural exception.",
        replace: "    /// Whether the interrupted code can be resumed.\n    pub const fn is_resumable(self) -> bool {\n        matches!(self, Self::Fault(Fault::Breakpoint))\n    }\n}\n\n/// An architectural exception.",
        breaks: "a speculative trap-resumption policy re-enters the public kernel boundary",
        reddens: Caught::ApiAudit("method impl::is_resumable"),
    },
    Mutation {
        id: 575,
        file: "libs/eevdf/src/lib.rs",
        find: "            Some((_, deadline)) if !before(entity.deadline, deadline) => {}",
        replace: "            Some((_, _deadline)) if false => {}",
        breaks: "a later eligible entity replaces the earliest deadline",
        reddens: Caught::HostTest {
            package: "eevdf",
            name: "tests::a_later_candidate_cannot_replace_an_earlier_deadline",
        },
    },
    Mutation {
        id: 576,
        file: "libs/elf64/src/lib.rs",
        find: "            Self::NotAnElf => core::fmt::Write::write_str(f, \"does not begin with the elf magic\"),",
        replace: "            Self::NotAnElf => core::fmt::Write::write_str(f, \"\"),",
        breaks: "an ELF magic refusal loses its diagnostic reason",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::diagnostic_messages_keep_their_reason_and_values",
        },
    },
    Mutation {
        id: 577,
        file: "libs/elf64/src/lib.rs",
        find: "    pub const fn flags(self) -> u32 {\n        self.flags\n    }",
        replace: "    pub const fn flags(self) -> u32 {\n        0\n    }",
        breaks: "ELF segment metadata erases every permission bit",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::segment_metadata_and_permission_boundaries_are_exact",
        },
    },
    Mutation {
        id: 578,
        file: "libs/elf64/src/lib.rs",
        find: "    /// Where in the file its contents start.\n    pub const fn offset(self) -> u64 {\n        self.offset\n    }",
        replace: "    /// Where in the file its contents start.\n    pub const fn offset(self) -> u64 {\n        0\n    }",
        breaks: "ELF segment metadata erases the file offset",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::segment_metadata_and_permission_boundaries_are_exact",
        },
    },
    Mutation {
        id: 579,
        file: "libs/elf64/src/lib.rs",
        find: "        self.flags & Self::READ != 0",
        replace: "        self.flags | Self::READ != 0",
        breaks: "every ELF segment is reported readable",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::segment_metadata_and_permission_boundaries_are_exact",
        },
    },
    Mutation {
        id: 580,
        file: "libs/elf64/src/lib.rs",
        find: "        self.file_size > self.memory_size",
        replace: "        self.file_size >= self.memory_size",
        breaks: "an ELF segment whose contents exactly fit is rejected as oversized",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::segment_metadata_and_permission_boundaries_are_exact",
        },
    },
    Mutation {
        id: 581,
        file: "libs/elf64/src/lib.rs",
        find: "    pub const fn segment_count(&self) -> u16 {\n        self.program_count\n    }",
        replace: "    pub const fn segment_count(&self) -> u16 {\n        1\n    }",
        breaks: "every ELF image reports one program segment",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::segment_metadata_and_permission_boundaries_are_exact",
        },
    },
    Mutation {
        id: 582,
        file: "libs/pixel/src/lib.rs",
        find: "            Self::Red => core::fmt::Write::write_str(f, \"red\"),",
        replace: "            Self::Red => core::fmt::Write::write_str(f, \"\"),",
        breaks: "pixel diagnostics omit the red channel name",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::diagnostic_messages_keep_their_channel_geometry_and_values",
        },
    },
    Mutation {
        id: 583,
        file: "libs/pixel/src/lib.rs",
        find: "            Self::EmptyChannel(channel) => write!(f, \"{channel} channel has no bits\"),",
        replace: "            Self::EmptyChannel(_) => f.write_str(\"\"),",
        breaks: "an empty pixel channel loses its diagnostic reason",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::diagnostic_messages_keep_their_channel_geometry_and_values",
        },
    },
    Mutation {
        id: 584,
        file: "libs/pixel/src/lib.rs",
        find: "            Self::ZeroWidth => core::fmt::Write::write_str(f, \"framebuffer width is zero\"),",
        replace: "            Self::ZeroWidth => core::fmt::Write::write_str(f, \"\"),",
        breaks: "a zero-width framebuffer loses its diagnostic reason",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::diagnostic_messages_keep_their_channel_geometry_and_values",
        },
    },
    Mutation {
        id: 585,
        file: "libs/pixel/src/lib.rs",
        find: "    let bits = if size == u32::BITS {\n        u32::MAX\n    } else {\n        (1 << size) - 1\n    };",
        replace: "    let bits = if size == u32::BITS {\n        u32::MAX\n    } else {\n        (1 << size) / 1\n    };",
        breaks: "a one-bit pixel-channel overlap disappears from its mask",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::a_one_bit_channel_overlap_is_still_an_overlap",
        },
    },
    Mutation {
        id: 586,
        file: "libs/port-space/src/lib.rs",
        find: "    left.start < right.end && right.start < left.end",
        replace: "    left.start < right.end && right.start <= left.end",
        breaks: "a port span touching above another is mistaken for overlap",
        reddens: Caught::HostTest {
            package: "port-space",
            name: "tests::a_request_that_overlaps_itself_is_refused",
        },
    },
    Mutation {
        id: 587,
        file: "libs/slab-layout/src/lib.rs",
        find: "                write!(f, \"alignment {align} is not a non-zero power of two\")",
        replace: "                f.write_str(\"\")",
        breaks: "a slab placement refusal loses the invalid alignment",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "tests::placement_errors_name_the_invalid_alignment",
        },
    },
    Mutation {
        id: 588,
        file: "libs/slab-layout/src/lib.rs",
        find: "            needed.next_power_of_two()",
        replace: "            needed.next_power_of_two() / 2",
        breaks: "a slab request rounds into the smaller size class",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "tests::every_slab_size_takes_the_exact_smallest_class",
        },
    },
    Mutation {
        id: 589,
        file: "libs/handle-space/src/lib.rs",
        find: "    pub const fn is_identity(self) -> bool {\n        self.0 == 0\n    }",
        replace: "    pub const fn is_identity(self) -> bool {\n        false\n    }",
        breaks: "handle mixer identity detection ignores the mixer bits",
        reddens: Caught::HostTest {
            package: "handle-space",
            name: "tests::identity_detection_reads_the_mixer_bits",
        },
    },
    Mutation {
        id: 590,
        file: "libs/handle-space/src/lib.rs",
        find: "    pub const fn union(self, other: Self) -> Self {\n        Self(self.0 | other.0)\n    }",
        replace: "    pub const fn union(self, other: Self) -> Self {\n        Self(self.0 ^ other.0)\n    }",
        breaks: "unioning a held handle right removes it",
        reddens: Caught::HostTest {
            package: "handle-space",
            name: "rights_tests::attenuation_only_narrows",
        },
    },
    Mutation {
        id: 591,
        file: "libs/memory-map/src/lib.rs",
        find: "                region.len != 0\n                    && region.base < end\n                    && start < region.end()\n                    && !allowed(region.kind)",
        replace: "                region.len != 0\n                    && region.base <= end\n                    && start < region.end()\n                    && !allowed(region.kind)",
        breaks: "a disallowed region beginning at the exclusive end is mistaken for overlap",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::a_disallowed_region_at_the_exclusive_end_does_not_overlap",
        },
    },
    Mutation {
        id: 600,
        file: "libs/elf64/src/lib.rs",
        find: "    pub const fn has_valid_alignment(self) -> bool {\n        self.alignment <= 1\n            || self.alignment.is_power_of_two()\n                && self.address % self.alignment == self.offset % self.alignment\n    }",
        replace: "    pub const fn has_valid_alignment(self) -> bool {\n        true\n    }",
        breaks: "ELF accepts non-power-of-two and file-incongruent segment alignment",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::segment_alignment_is_power_of_two_and_congruent",
        },
    },
    Mutation {
        id: 611,
        file: "libs/x86-cpuid/src/lib.rs",
        find: "    if speculation.rdcl_no() {",
        replace: "    if false && speculation.rdcl_no() {",
        breaks: "RDCL_NO no longer admits the shared kernel mapping policy",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::shared_kernel_mappings_need_rdcl_no_or_exact_qemu_tcg",
        },
    },
    Mutation {
        id: 612,
        file: "libs/x86-cpuid/src/lib.rs",
        find: "        ExecutionEnvironmentKind::QemuTcg => Some(SharedKernelMappingReason {\n            kind: SharedKernelMappingReasonKind::QemuTcg,\n        }),",
        replace: "        ExecutionEnvironmentKind::QemuTcg => None,",
        breaks: "the exact QEMU TCG exception no longer admits shared mappings",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::shared_kernel_mappings_need_rdcl_no_or_exact_qemu_tcg",
        },
    },
    Mutation {
        id: 613,
        file: "libs/x86-cpuid/src/lib.rs",
        find: "        if leaf_1_ecx & HYPERVISOR_BIT == 0 {",
        replace: "        if false && leaf_1_ecx & HYPERVISOR_BIT == 0 {",
        breaks: "the hypervisor leaf is read without CPUID proving it exists",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::the_hypervisor_leaf_is_not_read_without_the_feature_bit",
        },
    },
    Mutation {
        id: 614,
        file: "libs/x86-cpuid/src/lib.rs",
        find: "                u32::from_le_bytes(*b\"GTCG\"),",
        replace: "                u32::from_le_bytes(*b\"GTCF\"),",
        breaks: "a near-miss hypervisor identity is accepted as QEMU TCG",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::only_exact_qualified_hypervisor_leaves_authenticate_dma_handoff",
        },
    },
    Mutation {
        id: 615,
        file: "libs/x86-cpuid/src/lib.rs",
        find: "            SharedKernelMappingReasonKind::QemuTcg => \"qemu_tcg\",",
        replace: "            SharedKernelMappingReasonKind::QemuTcg => \"qemu\",",
        breaks: "the consumed QEMU TCG policy marker loses its stable name",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::shared_mapping_reasons_have_stable_policy_names",
        },
    },
    Mutation {
        id: 616,
        file: "libs/x86-cpuid/src/lib.rs",
        find: "const HYPERVISOR_BIT: u32 = 1 << 31;",
        replace: "const HYPERVISOR_BIT: u32 = 1 << 30;",
        breaks: "the hypervisor-present CPUID bit drifts from the architecture manual",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::the_speculation_constants_are_the_manual_s",
        },
    },
    Mutation {
        id: 638,
        file: "libs/memory-map/src/lib.rs",
        find: "        matches!(self, Self::Usable)",
        replace: "        false",
        breaks: "usable memory loses its exact allocator policy",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::every_memory_kind_has_its_exact_policy_and_name",
        },
    },
    Mutation {
        id: 639,
        file: "libs/memory-map/src/lib.rs",
        find: "                let overlaps = region.len != 0 && start < region.end() && region.base < end;",
        replace: "                let overlaps = region.len != 0 && start < region.end() && region.base <= end;",
        breaks: "a device extent overlaps a RAM region beginning at its exclusive end",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::a_disallowed_region_at_a_device_extents_exclusive_end_is_disjoint",
        },
    },
    Mutation {
        id: 640,
        file: "libs/elf64/src/lib.rs",
        find: "        if u32::from(identification_version) != CURRENT_VERSION {",
        replace: "        if false && u32::from(identification_version) != CURRENT_VERSION {",
        breaks: "ELF accepts an invalid identification version",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::every_required_header_identity_field_is_validated",
        },
    },
    Mutation {
        id: 641,
        file: "libs/elf64/src/lib.rs",
        find: "        if kind != EXECUTABLE {",
        replace: "        if false && kind != EXECUTABLE {",
        breaks: "the loader accepts a non-executable ELF object type",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::every_required_header_identity_field_is_validated",
        },
    },
    Mutation {
        id: 642,
        file: "libs/elf64/src/lib.rs",
        find: "        if version != CURRENT_VERSION {",
        replace: "        if false && version != CURRENT_VERSION {",
        breaks: "ELF accepts an invalid file-header version",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::every_required_header_identity_field_is_validated",
        },
    },
    Mutation {
        id: 643,
        file: "libs/elf64/src/lib.rs",
        find: "        if header_size as usize != HEADER_BYTES {",
        replace: "        if false && header_size as usize != HEADER_BYTES {",
        breaks: "ELF accepts a non-ELF64 file-header size",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::every_required_header_identity_field_is_validated",
        },
    },
    Mutation {
        id: 644,
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    repository_authority(&location[..authority_end])",
        replace: "    true",
        breaks: "a hostless HTTPS repository satisfies compiler provenance",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::repository_provenance_requires_a_host_bearing_https_authority",
        },
    },
    Mutation {
        id: 645,
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    repository_authority(&location[..authority_end])",
        replace: "    false",
        breaks: "every supported HTTPS repository authority is refused",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::repository_provenance_accepts_each_supported_host_form",
        },
    },
    Mutation {
        id: 646,
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    if host.len() > 253 {",
        replace: "    if host.len() >= 253 {",
        breaks: "the maximum valid DNS host length is refused",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::repository_host_length_boundary_is_exact",
        },
    },
    Mutation {
        id: 663,
        file: "libs/x86-apic/src/vector.rs",
        find: "        if vector < FIRST_PIC_VECTOR {",
        replace: "        if vector < FIRST_PIC_VECTOR && vector != 0x1f {",
        breaks: "external APIC vectors admit frame shapes owned by exception entry",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "vector::tests::every_cpu_vector_is_reserved",
        },
    },
    Mutation {
        id: 664,
        file: "libs/x86-apic/src/vector.rs",
        find: "            while other < index {",
        replace: "            while other < index.saturating_sub(1) {",
        breaks: "two APIC sources can be assigned one indistinguishable external vector",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "vector::tests::every_pair_is_accepted_exactly_when_assignable_and_distinct",
        },
    },
    Mutation {
        id: 689,
        file: "libs/ring-proto/src/lib.rs",
        find: "        if self.reserved + self.completion.occupied() >= self.completion.capacity().entries() {",
        replace: "        if true || self.reserved + self.completion.occupied() >= self.completion.capacity().entries() {",
        breaks: "the scheduled ring proof can pass when production admission always refuses",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "scheduled_calls_preserve_the_production_reservation",
        },
    },
    Mutation {
        id: 707,
        file: "libs/nvme/src/lib.rs",
        find: "        if namespace == 0 || !Self::contiguous_extent(address, IDENTIFY_TRANSFER_BYTES, page_size) {",
        replace: "        if namespace == 0 {",
        breaks: "identify can bind a partial, unaligned or wrapping DMA page",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::identify_binds_one_complete_aligned_page",
        },
    },
    Mutation {
        id: 708,
        file: "libs/nvme/src/lib.rs",
        find: "            || !Self::contiguous_extent(address, transfer.bytes(), page_size)",
        replace: "            || false",
        breaks: "read can bind bytes outside its validated contiguous DMA page",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::read_binds_its_validated_extent_to_one_page",
        },
    },
    Mutation {
        id: 709,
        file: "libs/nvme/src/lib.rs",
        find: "        if queue == 0 || !Self::contiguous_extent(address, capacity.completion_bytes(), page_size) {",
        replace: "        if queue == 0 {",
        breaks: "completion-queue creation can bind an incomplete contiguous extent",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::queue_commands_bind_the_complete_contiguous_extent",
        },
    },
    Mutation {
        id: 710,
        file: "libs/nvme/src/lib.rs",
        find: "        self.state = DmaOwnership::InFlight;\n        Some(handoff(DmaPublication { _state: self }))",
        replace: "        let outcome = handoff(DmaPublication { _state: self });\n        self.state = DmaOwnership::InFlight;\n        Some(outcome)",
        breaks: "doorbell handoff observes DMA memory before publication closes cancellation",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::publication_closes_the_unpublished_cancellation_door",
        },
    },
    Mutation {
        id: 711,
        file: "libs/nvme/src/lib.rs",
        find: "        if self.state != DmaOwnership::InFlight {",
        replace: "        if self.state == DmaOwnership::Idle {",
        breaks: "DMA completion accepts memory that was never published to the device",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::publication_closes_the_unpublished_cancellation_door",
        },
    },
    Mutation {
        id: 717,
        file: "libs/x86-cpuid/src/xstate.rs",
        find: "pub const fn matches_permitted_configuration(configured: u64) -> bool {\n    configured == PERMITTED\n}",
        replace: "pub const fn matches_permitted_configuration(configured: u64) -> bool {\n    configured & PERMITTED == PERMITTED\n}",
        breaks: "extra live XCR0 components can escape the fixed save-area policy",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "xstate::tests::the_live_configuration_is_exactly_the_fixed_policy",
        },
    },
    Mutation {
        id: 733,
        file: "libs/x86-apic/src/local_vector.rs",
        find: "        if vector.get() == spurious.get() {",
        replace: "        if vector.get() == spurious.get() && false {",
        breaks: "a configured APIC delivery can select the spurious vector",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "command::tests::a_spurious_self_delivery_performs_no_fake_register_write",
        },
    },
    Mutation {
        id: 734,
        file: "libs/x86-apic/src/timer.rs",
        find: "    if current_count != 0 {",
        replace: "    if false {",
        breaks: "a running local timer can be unmasked under a new vector",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "timer::tests::refused_one_shot_plans_perform_no_fake_register_write",
        },
    },
    Mutation {
        id: 735,
        file: "libs/x86-apic/src/in_service.rs",
        find: "    for (bank, bits) in registers.iter().enumerate().rev() {",
        replace: "    for (bank, bits) in registers.iter().enumerate() {",
        breaks: "EOI authority selects the lowest rather than highest in-service vector",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "in_service::tests::the_highest_set_vector_is_the_dismissal_target",
        },
    },
    Mutation {
        id: 744,
        file: "libs/x86-apic/src/in_service.rs",
        find: "pub fn owns(registers: &[u32; 8], vector: InterruptVector) -> bool {\n    highest(registers) == Some(vector.get())\n}",
        replace: "pub fn owns(registers: &[u32; 8], vector: InterruptVector) -> bool {\n    let _ = vector;\n    highest(registers).is_some()\n}",
        breaks: "a wrong-source in-service vector can authorize an EOI write",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "in_service::tests::absent_and_wrong_sources_plan_no_fake_eoi_write",
        },
    },
    Mutation {
        id: 745,
        file: "libs/x86-apic/src/in_service.rs",
        find: "    matches!(interrupts, Interrupts::Excluded)",
        replace: "    true",
        breaks: "EOI completion can write while maskable interrupts are enabled",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "in_service::tests::only_named_live_ownership_observations_authorize_completion",
        },
    },
    Mutation {
        id: 746,
        file: "libs/x86-apic/src/in_service.rs",
        find: "        && matches!(frame_liveness, FrameLiveness::Live)",
        replace: "        && true",
        breaks: "EOI completion accepts a token whose trap frame is no longer live",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "in_service::tests::only_named_live_ownership_observations_authorize_completion",
        },
    },
    Mutation {
        id: 747,
        file: "libs/x86-apic/src/in_service.rs",
        find: "        && frame_vector == token_vector.get() as u64",
        replace: "        && true",
        breaks: "EOI completion accepts a token from another trap vector",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "in_service::tests::only_named_live_ownership_observations_authorize_completion",
        },
    },
    Mutation {
        id: 748,
        file: "libs/x86-apic/src/in_service.rs",
        find: "        && matches!(dismissal, DismissalOwnership::Owned)",
        replace: "        && true",
        breaks: "EOI completion ignores which vector the APIC would dismiss",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "in_service::tests::only_named_live_ownership_observations_authorize_completion",
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
        id: 765,
        file: "libs/x86-apic/src/lifecycle.rs",
        find: "            Self::Cold => Ok(()),",
        replace: "            Self::Cold => Err(BeginError::Ready),",
        breaks: "a cold local APIC refuses its sole initialization transition",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "lifecycle::tests::only_cold_hardware_can_begin",
        },
    },
    Mutation {
        id: 766,
        file: "libs/x86-apic/src/lifecycle.rs",
        find: "            Ok(()) => Ok(Self {\n                lifecycle: Lifecycle::Initializing,\n                next_stage: 0,\n            }),",
        replace: "            Ok(()) => Ok(Self {\n                lifecycle: Lifecycle::Ready,\n                next_stage: 0,\n            }),",
        breaks: "an APIC initialization transaction publishes readiness before its first hardware boundary",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "lifecycle::tests::every_intermediate_observation_refuses_reentry_and_readiness",
        },
    },
    Mutation {
        id: 767,
        file: "libs/x86-apic/src/lifecycle.rs",
        find: "    pub fn refuse(&mut self) -> Lifecycle {\n        if matches!(self.lifecycle, Lifecycle::Initializing) {\n            self.lifecycle = Lifecycle::Poisoned;\n        }\n        self.lifecycle\n    }",
        replace: "    pub fn refuse(&mut self) -> Lifecycle {\n        if matches!(self.lifecycle, Lifecycle::Initializing) {\n            self.lifecycle = Lifecycle::Cold;\n        }\n        self.lifecycle\n    }",
        breaks: "a late APIC initialization refusal becomes retryable instead of terminal",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "lifecycle::tests::every_late_refusal_is_unpublished_and_terminal",
        },
    },
    Mutation {
        id: 768,
        file: "libs/x86-apic/src/lifecycle.rs",
        find: "const fn progress_is_excluded(interrupts: Interrupts) -> bool {\n    matches!(interrupts, Interrupts::Excluded)\n}",
        replace: "const fn progress_is_excluded(interrupts: Interrupts) -> bool {\n    let _ = interrupts;\n    true\n}",
        breaks: "an APIC hardware stage or publication can continue after interrupt exclusion is lost",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "lifecycle::tests::every_hardware_boundary_and_publication_require_interrupt_exclusion",
        },
    },
    Mutation {
        id: 769,
        file: "libs/x86-apic/src/lifecycle.rs",
        find: "        if InitStage::ALL.get(self.next_stage) != Some(&stage) {",
        replace: "        if false {",
        breaks: "APIC hardware initialization stages can run out of order",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "lifecycle::tests::publication_requires_every_stage_in_exact_order",
        },
    },
    Mutation {
        id: 770,
        file: "libs/x86-apic/src/lifecycle.rs",
        find: "        if self.next_stage != InitStage::ALL.len() {",
        replace: "        if false {",
        breaks: "APIC readiness can publish before every hardware stage completes",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "lifecycle::tests::publication_requires_every_stage_in_exact_order",
        },
    },
    Mutation {
        id: 777,
        file: "libs/x86-paging/src/lib.rs",
        find: "        if is_canonical(address) {",
        replace: "        if true {",
        breaks: "a safe page-invalidation address can cross either noncanonical hole boundary",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::canonical_address_admission_covers_both_sides_of_the_hole",
        },
    },
    Mutation {
        id: 778,
        file: "libs/x86-paging/src/activation.rs",
        find: "            2 => Some(Self::Ready),",
        replace: "            2 => None,",
        breaks: "the published paging-ready byte has no protocol state",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "activation::tests::every_byte_decodes_to_one_state_or_refusal",
        },
    },
    Mutation {
        id: 779,
        file: "libs/x86-paging/src/activation.rs",
        find: "            Self::Initializing => Some(Self::Ready),",
        replace: "            Self::Initializing => None,",
        breaks: "paging activation cannot settle its initializing transaction as ready",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "activation::tests::only_the_protocol_edges_are_admitted",
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
        id: 827,
        file: "libs/x86-paging/src/lib.rs",
        find: "        && span == PAGE_SIZE",
        replace: "        && span != 0",
        breaks: "page identity accepts a non-page leaf span",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::page_identity_ignores_only_hardware_observation_bits",
        },
    },
    Mutation {
        id: 830,
        file: "libs/nvme/src/lib.rs",
        find: "#[derive(Debug, PartialEq, Eq)]\npub struct DmaState {",
        replace: "#[derive(Debug, Clone, PartialEq, Eq)]\npub struct DmaState {",
        breaks: "DMA ownership authority can be duplicated before publication or completion",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma_ownership_and_publication_authorities_are_linear",
        },
    },
    Mutation {
        id: 831,
        file: "libs/ring-proto/src/lib.rs",
        find: "            if end > bytes {",
        replace: "            if end >= bytes {",
        breaks: "a publication word ending exactly at the allocation boundary is refused",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "tests::publication_words_exhaust_every_small_offset",
        },
    },
    Mutation {
        id: 832,
        file: "libs/ring-proto/src/lib.rs",
        find: "            let word_end = word + PUBLICATION_WORD_BYTES;",
        replace: "            let word_end = word;",
        breaks: "a payload byte can overlap the beginning of a publication word",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "tests::payload_exhausts_touching_overlapping_and_outside_ranges",
        },
    },
    Mutation {
        id: 833,
        file: "libs/ring-proto/src/lib.rs",
        find: "            if !at.is_multiple_of(align_of::<u32>()) {",
        replace: "            if false && !at.is_multiple_of(align_of::<u32>()) {",
        breaks: "a publication word can begin at a misaligned byte offset",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "tests::malformed_publication_partitions_are_refused_exactly",
        },
    },
    Mutation {
        id: 841,
        file: "libs/pixel/src/lib.rs",
        find: "        Some(RowCopyPlan {\n            source_word,\n            destination_word,\n            destination_offset,\n            row_words: self.width,\n            stride_words,",
        replace: "        Some(RowCopyPlan {\n            source_word,\n            destination_word,\n            destination_offset,\n            row_words: stride_words,\n            stride_words,",
        breaks: "a padded row copy publishes the row padding as visible pixels",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::row_copy_uses_padded_starts_without_publishing_padding",
        },
    },
    Mutation {
        id: 842,
        file: "libs/pixel/src/lib.rs",
        find: "        if to_y > from_y {",
        replace: "        if to_y < from_y {",
        breaks: "a framebuffer row-copy plan admits downward overlap and refuses upward movement",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::row_copy_refuses_downward_and_extreme_empty_ranges",
        },
    },
    Mutation {
        id: 843,
        file: "libs/pixel/src/lib.rs",
        find: "        self.source_end <= len && self.destination_end <= len",
        replace: "        self.destination_end <= len",
        breaks: "a row copy can read beyond a short framebuffer shadow",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::row_copy_refuses_a_short_shadow_without_side_effects",
        },
    },
    Mutation {
        id: 844,
        file: "libs/pixel/src/lib.rs",
        find: "            self.destination_offset += self.pitch;",
        replace: "            self.destination_offset += self.row_words * WORD_SIZE;",
        breaks: "a padded row copy advances device writes by visible width instead of pitch",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::every_small_row_copy_stays_within_its_padded_layout",
        },
    },
    Mutation {
        id: 845,
        file: "libs/x86-apic/src/lifecycle.rs",
        find: "/// One ordered boundary in the hardware transaction.\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub enum InitStage {",
        replace: "impl Interrupts {\n    pub const fn from_enabled(enabled: bool) -> Self {\n        if enabled { Self::Enabled } else { Self::Excluded }\n    }\n}\n\n/// One ordered boundary in the hardware transaction.\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub enum InitStage {",
        breaks: "APIC interrupt-exclusion policy regains a boolean constructor",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic_policy_observations_are_typed",
        },
    },
    Mutation {
        id: 846,
        file: "libs/x86-apic/src/in_service.rs",
        find: "pub const fn may_complete(\n    interrupts: Interrupts,\n    frame_liveness: FrameLiveness,\n    frame_vector: u64,\n    token_vector: InterruptVector,\n    dismissal: DismissalOwnership,\n) -> bool {\n    matches!(interrupts, Interrupts::Excluded)\n        && matches!(frame_liveness, FrameLiveness::Live)\n        && frame_vector == token_vector.get() as u64\n        && matches!(dismissal, DismissalOwnership::Owned)\n}",
        replace: "pub const fn may_complete(\n    interrupts_excluded: bool,\n    frame_is_live: bool,\n    frame_vector: u64,\n    token_vector: InterruptVector,\n    dismissal_owned: bool,\n) -> bool {\n    interrupts_excluded\n        && frame_is_live\n        && frame_vector == token_vector.get() as u64\n        && dismissal_owned\n}",
        breaks: "APIC completion collapses three independent observations into positional booleans",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic_policy_observations_are_typed",
        },
    },
    Mutation {
        id: 854,
        file: "libs/trap-cause/src/lib.rs",
        find: "        self.0 & !0b11 == 0",
        replace: "        self.0 & !0b111 == 0",
        breaks: "a user-origin page fault is classified as supervisor data access",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::only_exact_supervisor_data_faults_are_admitted",
        },
    },
    Mutation {
        id: 855,
        file: "libs/x86-paging/src/lib.rs",
        find: "    pub const ZERO: Self = Self(0);",
        replace: "    pub const ZERO: Self = Self(1);",
        breaks: "the canonical zero-address authority names a nonzero byte",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::canonical_address_admission_covers_both_sides_of_the_hole",
        },
    },
    Mutation {
        id: 856,
        file: "libs/x86-cpuid/src/lib.rs",
        find: "    pub const fn none() -> Self {\n        Self(0)\n    }",
        replace: "    pub const fn none() -> Self {\n        Self(1 << 11)\n    }",
        breaks: "an empty CR4 protection update silently enables UMIP",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::cr4_updates_are_add_only_and_exact",
        },
    },
    Mutation {
        id: 857,
        file: "libs/x86-cpuid/src/lib.rs",
        find: "        Self(self.0 | 1 << 20)",
        replace: "        Self(self.0 | 1 << 21)",
        breaks: "the SMEP builder enables SMAP instead of execution prevention",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::cr4_updates_are_add_only_and_exact",
        },
    },
    Mutation {
        id: 858,
        file: "libs/x86-cpuid/src/lib.rs",
        find: "        Self(self.0 | 1 << 21)",
        replace: "        Self(self.0 | 1 << 20)",
        breaks: "the SMAP builder enables SMEP instead of access prevention",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::cr4_updates_are_add_only_and_exact",
        },
    },
    Mutation {
        id: 859,
        file: "libs/x86-cpuid/src/lib.rs",
        find: "        Self(self.0 | 1 << 11)",
        replace: "        Self(self.0 | 1 << 21)",
        breaks: "the UMIP builder enables SMAP instead of instruction prevention",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::cr4_updates_are_add_only_and_exact",
        },
    },
    Mutation {
        id: 862,
        file: "libs/trap-cause/src/lib.rs",
        find: "        self.vector == u64::from(exception::PAGE_FAULT)",
        replace: "        self.vector <= u64::from(exception::PAGE_FAULT)",
        breaks: "controlled recovery accepts a non-page-fault vector",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::recovery_requires_the_page_fault_vector",
        },
    },
    Mutation {
        id: 863,
        file: "libs/trap-cause/src/lib.rs",
        find: "            && self.frame_error_code == self.page_fault.bits()",
        replace: "            && self.frame_error_code == self.frame_error_code",
        breaks: "controlled recovery does not bind the saved error word to its decoded fault",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::recovery_requires_the_error_code_the_dispatcher_decoded",
        },
    },
    Mutation {
        id: 864,
        file: "libs/trap-cause/src/lib.rs",
        find: "            && self.code_selector & 0b11 == 0",
        replace: "            && self.code_selector & 0b11 != 3",
        breaks: "controlled recovery admits ring-one and ring-two trap frames",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::recovery_requires_a_ring_zero_frame",
        },
    },
    Mutation {
        id: 865,
        file: "libs/trap-cause/src/lib.rs",
        find: "            RecoveryFault::SupervisorData => self.page_fault.is_supervisor_data_access(),",
        replace: "            RecoveryFault::SupervisorData => true,",
        breaks: "user-copy recovery accepts every page-fault class",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::user_copy_recovery_accepts_only_supervisor_data_faults",
        },
    },
    Mutation {
        id: 866,
        file: "libs/trap-cause/src/lib.rs",
        find: "            RecoveryFault::SupervisorWriteProtection => self.page_fault.bits() == 0b11,",
        replace: "            RecoveryFault::SupervisorWriteProtection => {\n                self.page_fault.is_supervisor_data_access()\n            }",
        breaks: "write-protection recovery accepts missing and read faults",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::write_protection_recovery_requires_the_exact_protection_write",
        },
    },
    Mutation {
        id: 867,
        file: "libs/trap-cause/src/lib.rs",
        find: "            && rule.fault_addresses.contains(&self.faulting_address)",
        replace: "            && self.faulting_address < rule.fault_addresses.end",
        breaks: "controlled recovery ignores the lower bound and emptiness of its CR2 interval",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::recovery_requires_cr2_inside_the_owned_interval",
        },
    },
    Mutation {
        id: 868,
        file: "libs/trap-cause/src/lib.rs",
        find: "            && rule\n                .instruction_addresses\n                .contains(&self.instruction_pointer)",
        replace: "            && self.instruction_pointer < rule.instruction_addresses.end",
        breaks: "controlled recovery ignores the lower bound and emptiness of its RIP interval",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::recovery_requires_rip_inside_the_owned_interval",
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
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 920,
        file: "libs/nvme/src/lib.rs",
        find: "pub const fn doorbell(cap: Capabilities, queue: u16, doorbell: Doorbell) -> Option<usize> {",
        replace: "pub const fn doorbell(cap: Capabilities, queue: u16, doorbell: bool) -> Option<usize> {",
        breaks: "NVMe submission/completion doorbell selection becomes positional boolean state",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 921,
        file: "libs/nvme/src/lib.rs",
        find: "    /// Permanently refuse another publication on this controller.\n    pub fn close(&mut self) {",
        replace: "    /// Recover hardware quiescence from an unnamed observation.\n    pub fn quiesce(&mut self, verified: bool) {\n        if verified {\n            self.close();\n        }\n    }\n\n    /// Permanently refuse another publication on this controller.\n    pub fn close(&mut self) {",
        breaks: "pure NVMe DMA state regains boolean hardware-quiescence authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 922,
        file: "libs/nvme/src/lib.rs",
        find: "    /// Permanently refuse another publication on this controller.\n    pub fn close(&mut self) {",
        replace: "    /// Recover retained hardware ownership from an unnamed observation.\n    pub fn retain_unquiesced(&mut self, retained: bool) {\n        if retained {\n            self.close();\n        }\n    }\n\n    /// Permanently refuse another publication on this controller.\n    pub fn close(&mut self) {",
        breaks: "pure NVMe DMA state regains boolean retention authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 923,
        file: "libs/nvme/src/lib.rs",
        find: "    pub fn close(&mut self) {",
        replace: "    pub fn close(&mut self, _closed: bool) {",
        breaks: "NVMe DMA closure regains a redundant unnamed state boolean",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 924,
        file: "libs/x86-cpuid/src/xstate.rs",
        find: "    pub const fn new(size: u32, alignment: Alignment) -> Self {",
        replace: "    pub const fn new(size: u32, alignment: bool) -> Self {",
        breaks: "xstate component alignment becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 957,
        file: "libs/nvme/src/lib.rs",
        find: "        self.state = DmaOwnership::Closed;",
        replace: "        self.state = DmaOwnership::Idle;",
        breaks: "closing pure DMA state reopens publication instead of making it terminal",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::closed_dma_state_cannot_publish_or_complete",
        },
    },
    Mutation {
        id: 986,
        file: "libs/nvme/src/lib.rs",
        find: "    pub fn close(&mut self) {",
        replace: "    pub fn quiesce(&mut self) {",
        breaks: "pure DMA state renames terminal closure into a hardware-recycling claim",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 996,
        file: "libs/acpi-tables/src/lib.rs",
        find: "fn checksum_is_valid(bytes: &[u8]) -> bool {",
        replace: "pub fn checksum_is_valid(bytes: &[u8]) -> bool {",
        breaks: "ACPI checksum validation regrows as speculative public surface",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 997,
        file: "libs/acpi-tables/src/lib.rs",
        find: "#[cfg(test)]\nfn fix_checksum(bytes: &mut [u8], index: usize) {",
        replace: "pub fn fix_checksum(bytes: &mut [u8], index: usize) {",
        breaks: "the test-only ACPI checksum forger becomes a production API",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 998,
        file: "libs/fat32/src/lib.rs",
        find: "    const fn sector_of(self, cluster: u32) -> Result<u64, FatError> {",
        replace: "    pub const fn sector_of(self, cluster: u32) -> Result<u64, FatError> {",
        breaks: "FAT cluster-to-sector internals regrow as speculative public arithmetic",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 999,
        file: "libs/fat32/src/lib.rs",
        find: "    const fn table_position(self, cluster: u32) -> (u64, usize) {",
        replace: "    pub const fn table_position(self, cluster: u32) -> (u64, usize) {",
        breaks: "FAT table-position internals regrow as speculative public arithmetic",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 1000,
        file: "libs/fat32/src/lib.rs",
        find: "const fn decode_entry(raw: u32) -> Next {",
        replace: "pub const fn decode_entry(raw: u32) -> Next {",
        breaks: "raw FAT entry decoding regrows as a speculative public policy door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 1001,
        file: "libs/fat32/src/lib.rs",
        find: "const fn decode_slot(slot: &[u8; ENTRY_BYTES]) -> Slot {",
        replace: "pub const fn decode_slot(slot: &[u8; ENTRY_BYTES]) -> Slot {",
        breaks: "raw FAT directory-slot decoding regrows as a speculative public policy door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 1002,
        file: "libs/memory-map/src/lib.rs",
        find: "pub const MAX_REGIONS: usize = 128;",
        replace: "pub const MAX_REGIONS: usize = 128;\n\npub fn allocatable_bytes() -> u64 {\n    0\n}",
        breaks: "memory-map allocation policy regrows a speculative aggregate API",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 1003,
        file: "libs/path-walk/src/lib.rs",
        find: "fn is_a_name(component: &[u8]) -> bool {",
        replace: "pub fn is_a_name(component: &[u8]) -> bool {",
        breaks: "pathname component policy regrows as a speculative public predicate",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
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
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 1006,
        file: "libs/stack-layout/src/lib.rs",
        find: "pub const PAGE_SIZE: u64 = 4096;",
        replace: "pub const PAGE_SIZE: u64 = 4096;\n\nfn stack_overlaps() -> bool {\n    false\n}",
        breaks: "stack layout regrows a second speculative overlap policy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
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
            name: "contracts::pci_configuration_writes_require_function_authority",
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
            name: "contracts::pci_configuration_writes_require_function_authority",
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
            name: "contracts::pci_configuration_writes_require_function_authority",
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
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        id: 1152,
        file: "libs/acpi-tables/src/lib.rs",
        find: "        if listed > MAX_TABLES {",
        replace: "        if false {",
        breaks: "an over-budget ACPI root table can hide a tail pointer",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_root_table_above_the_cap_cannot_hide_a_tail_madt",
        },
    },
    Mutation {
        id: 1153,
        file: "libs/acpi-tables/src/lib.rs",
        find: "        if listed > MAX_TABLES {",
        replace: "        if listed >= MAX_TABLES {",
        breaks: "an ACPI root table at the exact pointer budget is refused",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_root_table_at_the_cap_is_walked_whole",
        },
    },
    Mutation {
        id: 1154,
        file: "libs/acpi-tables/src/lib.rs",
        find: "            8 => u64_at(self.body, self.offset)?,",
        replace: "            8 => u32_at(self.body, self.offset)? as u64,",
        breaks: "XSDT iteration truncates every table pointer to thirty-two bits",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::an_xsdt_pointer_keeps_its_high_thirty_two_bits",
        },
    },
    Mutation {
        id: 1155,
        file: "libs/acpi-tables/src/lib.rs",
        find: "        if self.revision >= 2 && self.xsdt != 0 {",
        replace: "        if self.revision >= 2 || self.xsdt != 0 {",
        breaks: "an XSDT pointer is published without both revision and nonzero-address proof",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::an_xsdt_requires_both_a_supporting_revision_and_a_nonzero_address",
        },
    },
    Mutation {
        id: 1156,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                enabled: u32_at(entry, 4).unwrap_or(0) & 1 != 0,",
        replace: "                enabled: u32_at(entry, 4).unwrap_or(0) & 1 == 0,",
        breaks: "a disabled local APIC is reported enabled and an enabled one disabled",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_local_apic_without_the_enabled_bit_is_disabled",
        },
    },
    Mutation {
        id: 1157,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    if bus != 0 {",
        replace: "                    if false {",
        breaks: "a non-ISA interrupt override enters the ISA routing inventory",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::interrupt_overrides_are_unique_isa_sources_with_defined_flags",
        },
    },
    Mutation {
        id: 1158,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    let Some(slot) = model.overrides.get_mut(source as usize) else {",
        replace: "                    let Some(slot) = model.overrides.get_mut((source as usize).min(15)) else {",
        breaks: "an interrupt override outside the ISA source domain aliases a valid source",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::interrupt_overrides_are_unique_isa_sources_with_defined_flags",
        },
    },
    Mutation {
        id: 1159,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    if flags & !0xf != 0 || polarity == 0b10 || trigger == 0b10 {",
        replace: "                    if false {",
        breaks: "reserved interrupt polarity and trigger encodings enter the routing inventory",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::interrupt_overrides_are_unique_isa_sources_with_defined_flags",
        },
    },
    Mutation {
        id: 1160,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    if slot.is_some() {",
        replace: "                    if false {",
        breaks: "a later interrupt override silently replaces the same ISA source",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::interrupt_overrides_are_unique_isa_sources_with_defined_flags",
        },
    },
    Mutation {
        id: 1179,
        file: "libs/nvme/src/lib.rs",
        find: "#[derive(Debug, PartialEq, Eq)]\npub struct DmaState {",
        replace: "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub struct DmaState {",
        breaks: "DMA ownership state can be copied as well as cloned",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma_ownership_and_publication_authorities_are_linear",
        },
    },
    Mutation {
        id: 1180,
        file: "libs/nvme/src/lib.rs",
        find: "#[derive(Debug)]\npub struct DmaPublication<'a> {",
        replace: "#[derive(Debug, Clone)]\npub struct DmaPublication<'a> {",
        breaks: "a device-publication proof can be cloned while its source remains live",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma_ownership_and_publication_authorities_are_linear",
        },
    },
    Mutation {
        id: 1181,
        file: "libs/nvme/src/lib.rs",
        find: "#[derive(Debug)]\npub struct DmaPublication<'a> {",
        replace: "#[derive(Debug, Clone, Copy)]\npub struct DmaPublication<'a> {",
        breaks: "a device-publication proof can be copied without consuming its authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::dma_ownership_and_publication_authorities_are_linear",
        },
    },
    Mutation {
        id: 1270,
        file: "libs/x86-apic/src/vector.rs",
        find: "        if vector >= FIRST_PIC_VECTOR && vector < FIRST_ASSIGNABLE {",
        replace: "        if vector >= FIRST_PIC_VECTOR && vector < FIRST_ASSIGNABLE && vector != 0x2f {",
        breaks: "the typed vector boundary admits the last remapped PIC slot",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "vector::tests::every_remapped_pic_vector_is_reserved",
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
        id: 1278,
        file: "libs/nvme/src/lib.rs",
        find: "        if cc & configuration::ENABLE != 0 || csts & status::READY != 0 {",
        replace: "        if csts & status::READY != 0 {",
        breaks: "NVMe quiescence ignores an enabled controller",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::only_disabled_and_not_ready_registers_pass_dma_classification",
        },
    },
    Mutation {
        id: 1279,
        file: "libs/nvme/src/lib.rs",
        find: "        if cc & configuration::ENABLE != 0 || csts & status::READY != 0 {",
        replace: "        if cc & configuration::ENABLE != 0 {",
        breaks: "NVMe quiescence ignores a controller that remains ready",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::only_disabled_and_not_ready_registers_pass_dma_classification",
        },
    },
    Mutation {
        id: 1280,
        file: "libs/nvme/src/lib.rs",
        find: "        match DisabledRegisters::from_bits(cc, csts) {",
        replace: "        match DisabledRegisters::from_bits(0, 0) {",
        breaks: "unverified NVMe register state can discard quarantine authority",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_unverified_preparation_access_failure_preserves_its_quarantine_owner",
        },
    },
    Mutation {
        id: 1281,
        file: "libs/nvme/src/lib.rs",
        find: "impl<T> DisabledPreparation<T> {\n    /// Attempt one disabled-controller access with exact retirement fallback.\n    pub fn access<U, E>(\n        mut self,\n        operation: impl FnOnce(&mut T) -> Result<U, E>,\n    ) -> Result<(Self, U), PreparationFailure<Self, E>> {\n        match operation(&mut self.owner) {\n            Ok(value) => Ok((self, value)),\n            Err(reason) => Err(PreparationFailure {\n                preparation: self,\n                reason,\n            }),\n        }\n    }",
        replace: "impl<T> DisabledPreparation<T> {\n    /// Attempt one disabled-controller access with exact retirement fallback.\n    pub fn access<U, E>(\n        mut self,\n        operation: impl FnOnce(&mut T) -> Result<U, E>,\n    ) -> Result<(Self, U), PreparationFailure<Self, E>> {\n        match operation(&mut self.owner) {\n            Ok(value) => Ok((self, value)),\n            Err(_) => panic!(\"mutant discarded disabled-controller cleanup authority\"),\n        }\n    }",
        breaks: "a disabled-controller access refusal discards exact retirement authority",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_disabled_preparation_access_failure_returns_the_exact_retirement_owner",
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
        id: 1296,
        file: "libs/memory-map/src/lib.rs",
        find: "        matches!(self, Self::Reserved | Self::Unknown(_))",
        replace: "        matches!(\n            self,\n            Self::Reserved | Self::Unknown(_) | Self::BootloaderReclaimable\n        )",
        breaks: "device admission treats bootloader storage as an MMIO hole",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::every_forbidden_kind_and_byte_in_a_page_refuses_device_mapping",
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
            name: "contracts::pci_bar_mapping_requires_exact_linear_authority",
        },
    },
    Mutation {
        id: 1299,
        file: "libs/x86-cpuid/src/physical.rs",
        find: "        if maximum_extended_leaf < ADDRESS_SIZE_LEAF || bits < 32 || bits > 52 {",
        replace: "        if bits < 32 || bits > 52 {",
        breaks: "an absent address-size leaf lends an unrelated register value as MAXPHYADDR",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "physical::tests::an_absent_leaf_never_borrows_an_unrelated_answer",
        },
    },
    Mutation {
        id: 1300,
        file: "libs/x86-cpuid/src/physical.rs",
        find: "        start < end && end <= 1_u64 << self.0",
        replace: "        start < end && end < 1_u64 << self.0",
        breaks: "the final implemented physical byte is rejected by MAXPHYADDR admission",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "physical::tests::each_supported_limit_is_exclusive",
        },
    },
    Mutation {
        id: 1301,
        file: "libs/x86-paging/src/device.rs",
        find: "        let page_start = physical - physical % PAGE_SIZE;",
        replace: "        let page_start = physical;",
        breaks: "device admission omits bytes before an unaligned register base in its first page",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "device::tests::a_short_register_range_expands_to_the_complete_page",
        },
    },
    Mutation {
        id: 1302,
        file: "libs/x86-paging/src/device.rs",
        find: "        if !width.contains(page_start, page_end) {",
        replace: "        if false {",
        breaks: "device admission ignores the running processor's physical-address width",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "device::tests::each_runtime_width_accepts_its_last_page_and_refuses_the_next",
        },
    },
    Mutation {
        id: 1314,
        file: "libs/x86-paging/src/lib.rs",
        find: "        self.address() == found.address()\n            && self.flags().difference(Flags::ACCESSED).bits()\n                == found.flags().difference(Flags::ACCESSED).bits()",
        replace: "        self.bits() == found.bits()",
        breaks: "binding rejects harmless hardware-accessed drift in a shared root",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::table_identity_ignores_only_accessed",
        },
    },
    Mutation {
        id: 1315,
        file: "libs/x86-paging/src/lib.rs",
        find: "        self.address() == found.address()\n            && self.flags().difference(Flags::ACCESSED).bits()\n                == found.flags().difference(Flags::ACCESSED).bits()",
        replace: "        self.address() == found.address()\n            && self\n                .flags()\n                .difference(Flags::ACCESSED.union(Flags::NO_CACHE))\n                .bits()\n                == found\n                    .flags()\n                    .difference(Flags::ACCESSED.union(Flags::NO_CACHE))\n                    .bits()",
        breaks: "binding ignores software-controlled cache-policy drift in a shared root",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::table_identity_ignores_only_accessed",
        },
    },
    Mutation {
        id: 1323,
        file: "libs/slab-layout/src/ownership.rs",
        find: "        self.free_frames\n            .checked_add(self.slab_frames)?\n            .checked_add(self.large_frames)",
        replace: "        Some(\n            self.free_frames\n                .wrapping_add(self.slab_frames)\n                .wrapping_add(self.large_frames),\n        )",
        breaks: "overflow wraps an allocator ownership account",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "ownership::tests::every_boundary_account_fails_closed_on_overflow",
        },
    },
    Mutation {
        id: 1324,
        file: "libs/slab-layout/src/ownership.rs",
        find: "                .is_some_and(|accounted| before.accounted_physical_frames() == Some(accounted))",
        replace: "                .is_some_and(|accounted| {\n                    before.accounted_physical_frames().unwrap_or(accounted) == accounted\n                })",
        breaks: "an overflowing earlier allocator account defaults to the later total",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "ownership::tests::every_boundary_account_fails_closed_on_overflow",
        },
    },
    Mutation {
        id: 1325,
        file: "libs/slab-layout/src/ownership.rs",
        find: "            && self\n                .accounted_physical_frames()\n                .is_some_and(|accounted| before.accounted_physical_frames() == Some(accounted))",
        replace: "            && self.accounted_physical_frames().is_some()",
        breaks: "allocator ownership accepts a changed physical-frame total",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "ownership::tests::every_small_free_and_slab_account_obeys_conservation",
        },
    },
    Mutation {
        id: 1326,
        file: "libs/slab-layout/src/ownership.rs",
        find: "        self.live_allocations == before.live_allocations\n            && self.slab_frames >= before.slab_frames",
        replace: "        self.slab_frames >= before.slab_frames",
        breaks: "allocator ownership ignores a changed live-allocation count",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "ownership::tests::live_count_is_exact_while_large_frame_ownership_may_move",
        },
    },
    Mutation {
        id: 1327,
        file: "libs/slab-layout/src/ownership.rs",
        find: "        self.live_allocations == before.live_allocations\n            && self.slab_frames >= before.slab_frames",
        replace: "        self.live_allocations == before.live_allocations\n            && self.slab_frames >= before.slab_frames\n            && self.large_frames == before.large_frames",
        breaks: "allocator ownership rejects a live allocation's capacity change",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "ownership::tests::live_count_is_exact_while_large_frame_ownership_may_move",
        },
    },
    Mutation {
        id: 1328,
        file: "libs/slab-layout/src/ownership.rs",
        find: "        self.live_allocations == before.live_allocations\n            && self.slab_frames >= before.slab_frames",
        replace: "        self.live_allocations == before.live_allocations",
        breaks: "allocator ownership accepts the disappearance of a retained slab",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "ownership::tests::every_conserved_slab_transition_is_directionally_monotonic",
        },
    },
    Mutation {
        id: 1383,
        file: "libs/fat32/src/lib.rs",
        find: "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\n#[non_exhaustive]\npub enum FatError {",
        replace: "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub enum FatError {",
        breaks: "a public error enum can regain an exhaustive downstream variant contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::public_error_enums_have_stable_standard_contracts",
        },
    },
    Mutation {
        id: 1384,
        file: "libs/fat32/src/lib.rs",
        find: "            Self::Unreadable => core::fmt::Write::write_str(f, \"the sector source refused a read\"),",
        replace: "            Self::Unreadable => core::fmt::Write::write_str(f, \"unreadable\"),",
        breaks: "a FAT refusal loses its actionable sector-source cause",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::every_fat_error_describes_its_cause",
        },
    },
    Mutation {
        id: 1385,
        file: "libs/path-walk/src/lib.rs",
        find: "                Self::Empty => \"the path is empty\",",
        replace: "                Self::Empty => \"empty\",",
        breaks: "a path refusal loses the argument it describes",
        reddens: Caught::HostTest {
            package: "path-walk",
            name: "tests::every_path_error_describes_its_cause",
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
        id: 1387,
        file: "libs/x86-apic/src/timer.rs",
        find: "        core::fmt::Write::write_str(f, \"the local APIC timer is still running\")",
        replace: "        core::fmt::Write::write_str(f, \"timer error\")",
        breaks: "a one-shot refusal no longer identifies the still-running timer",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "timer::tests::one_shot_error_is_a_standard_error",
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
            name: "contracts::public_failure_wrappers_are_must_use",
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
            name: "contracts::pci_configuration_writes_require_function_authority",
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
            name: "contracts::pci_function_authority_has_audited_mints_and_no_duplication_door",
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
            name: "contracts::pci_function_authority_has_audited_mints_and_no_duplication_door",
        },
    },
    Mutation {
        id: 1524,
        file: "libs/x86-paging/src/lib.rs",
        find: "        Some(end) => end <= USER_MAPPING_END,",
        replace: "        Some(end) => end <= USER_END,",
        breaks: "a user mapping may cover the last canonical page",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::no_user_mapping_reaches_the_last_canonical_page",
        },
    },
    Mutation {
        id: 1527,
        file: "libs/memory-map/src/lib.rs",
        find: "        self.covered_by(start, end, |kind| kind == MemoryKind::Framebuffer)\n            .then_some(DirectMapCache::Framebuffer)",
        replace: "        Some(DirectMapCache::Framebuffer)",
        breaks: "a boot range mixing framebuffer and ordinary memory gets one cache type",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::a_boot_range_has_one_direct_map_cache_type_or_none",
        },
    },
    Mutation {
        id: 1531,
        file: "libs/x86-apic/src/io.rs",
        find: "    let identity = version & 0xFF;\n    if identity < MIN_VERSION || identity > MAX_VERSION {\n        return None;\n    }",
        replace: "    let identity = version & 0xFF;\n    let _ = (identity, MIN_VERSION, MAX_VERSION);",
        breaks: "the I/O APIC version decoder admits any version byte",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "io::tests::a_version_register_yields_an_entry_count_only_when_it_is_plausible",
        },
    },
    Mutation {
        id: 1534,
        file: "libs/fat32/src/lib.rs",
        find: "                    if visited >= self.bpb.directory_cluster_limit() {",
        replace: "                    if visited >= self.chain_limit() {",
        breaks: "a directory walk is bounded by the volume, not the entry cap",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_huge_acyclic_directory_stops_at_the_entry_cap",
        },
    },
    Mutation {
        id: 1535,
        file: "libs/fat32/src/lib.rs",
        find: "                    let hare = self.advance_cycle_hare(cursor.hare)?;\n                    if hare == Some(next) {\n                        return Err(FatError::Unbounded);\n                    }\n",
        replace: "                    let hare = self.advance_cycle_hare(cursor.hare)?;\n",
        breaks: "a directory walk never checks its chain for a cycle",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_cyclic_directory_chain_is_caught_by_the_hare_not_the_volume",
        },
    },
    Mutation {
        id: 1540,
        file: "libs/acpi-tables/src/lib.rs",
        find: "dmar.entries().skip(index + 1)",
        replace: "dmar.entries().skip(index)",
        breaks: "the duplicate-unit scan compares each unit to itself, so a valid table is rejected",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_dmar_yields_its_structures_and_resolves_each_bdf_to_its_unit_and_regions",
        },
    },
    Mutation {
        id: 1541,
        file: "libs/acpi-tables/src/lib.rs",
        find: "DmarStructure::Rmrr { base, limit, .. } if base > limit",
        replace: "DmarStructure::Rmrr { base, limit, .. } if base < limit",
        breaks: "a reserved region whose base exceeds its limit is admitted instead of refused",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::an_inverted_reserved_region_is_refused_by_the_validated_view",
        },
    },
    Mutation {
        id: 1542,
        file: "libs/vtd/src/lib.rs",
        find: "    pub const TE: u32 = 1 << 31;",
        replace: "    pub const TE: u32 = 1 << 30;",
        breaks: "the translation-enable command bit is encoded at the wrong position",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::global_command_and_status_bits_match_the_spec",
        },
    },
    Mutation {
        id: 1543,
        file: "libs/vtd/src/lib.rs",
        find: "    const READ: u64 = 1 << 0;",
        replace: "    const READ: u64 = 1 << 1;",
        breaks: "a second-level entry's read permission collides with its write bit",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::second_level_entries_encode_permission_and_address",
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
        id: 1551,
        file: "libs/vtd/src/lib.rs",
        find: "        address >> self.address_bits() == 0",
        replace: "        address >> (self.address_bits() + 1) == 0",
        breaks: "an address one bit above the walk's width counts as covered",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::a_width_covers_every_address_below_its_power_of_two_and_none_above",
        },
    },
    Mutation {
        id: 1552,
        file: "libs/vtd/src/lib.rs",
        find: "status & (gsts::TES | gsts::QIES | gsts::IRES | gsts::CFIS) == 0",
        replace: "status & (gsts::TES | gsts::QIES) == 0",
        breaks: "taking the unit over inherits firmware interrupt and compatibility-bypass state",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::takeover_requires_every_firmware_facility_to_be_inactive",
        },
    },
    Mutation {
        id: 1553,
        file: "libs/vtd/src/lib.rs",
        find: "status & (gsts::TES | gsts::QIES | gsts::IRES | gsts::CFIS) == 0",
        replace: "status & (gsts::TES | gsts::IRES | gsts::CFIS) == 0",
        breaks: "takeover accepts a unit whose firmware invalidation queue is still live",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::takeover_requires_every_firmware_facility_to_be_inactive",
        },
    },
    Mutation {
        id: 1556,
        file: "libs/acpi-tables/src/lib.rs",
        find: "            bus = buses.secondary;\n",
        replace: "            bus = buses.subordinate;\n",
        breaks: "a scope's walk descends to a bridge's subordinate bus instead of its secondary",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::an_endpoint_scope_resolves_through_each_bridge_on_its_path",
        },
    },
    Mutation {
        id: 1557,
        file: "libs/acpi-tables/src/lib.rs",
        find: "        for (next_device, next_function) in hops {\n            let at = Bdf::new(segment, bus, device, function);",
        replace: "        for (next_device, next_function) in hops {\n            let at = Bdf::new(0, bus, device, function);",
        breaks: "a scope's walk looks its bridges up in segment zero instead of its own segment",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::an_endpoint_scope_resolves_through_each_bridge_on_its_path",
        },
    },
    Mutation {
        id: 1558,
        file: "libs/acpi-tables/src/lib.rs",
        find: "(scope.buses.secondary..=scope.buses.subordinate).contains(&bdf.bus())",
        replace: "(scope.buses.secondary..scope.buses.subordinate).contains(&bdf.bus())",
        breaks: "a bridge scope stops one bus short of its bridge's subordinate bus",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_bridge_scope_covers_its_bridge_and_exactly_the_buses_behind_it",
        },
    },
    Mutation {
        id: 1559,
        file: "libs/acpi-tables/src/lib.rs",
        find: "            Self::Hierarchy(scope) => {\n                scope.root == bdf\n                    || (scope.buses.secondary..=scope.buses.subordinate)",
        replace: "            Self::Hierarchy(scope) => {\n                (scope.buses.secondary..=scope.buses.subordinate)",
        breaks: "a bridge scope covers the buses behind its bridge but not the bridge itself",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_bridge_scope_covers_its_bridge_and_exactly_the_buses_behind_it",
        },
    },
    Mutation {
        id: 1560,
        file: "libs/acpi-tables/src/lib.rs",
        find: "            let buses = behind(at, bridge)?;\n            bus = buses.secondary;\n",
        replace: "            let buses = behind(at, bridge);\n            bus = buses.map_or(bus, |buses| buses.secondary);\n",
        breaks: "a hop that is not a bridge is stepped over as if the path went on along its bus",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_path_through_a_function_that_is_not_a_bridge_names_nothing",
        },
    },
    Mutation {
        id: 1561,
        file: "libs/acpi-tables/src/lib.rs",
        find: "    (at.bus() < buses.secondary && buses.secondary <= buses.subordinate).then_some(buses)",
        replace: "    (at.bus() <= buses.secondary && buses.secondary <= buses.subordinate).then_some(buses)",
        breaks: "a bridge whose secondary bus is its own bus is followed, so a walk can return to a bus",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_bridge_that_does_not_lead_past_its_own_bus_names_nothing",
        },
    },
    Mutation {
        id: 1562,
        file: "libs/acpi-tables/src/lib.rs",
        find: " && buses.secondary <= buses.subordinate).then_some(buses)",
        replace: ").then_some(buses)",
        breaks: "a bridge whose secondary bus exceeds its subordinate is walked as if its numbering held",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_bridge_whose_secondary_bus_exceeds_its_subordinate_names_nothing",
        },
    },
    Mutation {
        id: 1564,
        file: "libs/acpi-tables/src/lib.rs",
        find: "            DeviceScopeKind::IoApic\n            | DeviceScopeKind::Hpet\n            | DeviceScopeKind::AcpiNamespaceDevice\n            | DeviceScopeKind::Other { .. } => return None,",
        replace: "            _ => false,",
        breaks: "an I/O APIC, HPET, namespace-device or unknown scope is walked as a PCI endpoint",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::scopes_that_name_no_pci_function_never_govern_one",
        },
    },
    Mutation {
        id: 1565,
        file: "libs/acpi-tables/src/lib.rs",
        find: "        if endpoint.is_some() {",
        replace: "        if endpoint.is_some() && enclosing.is_none() {",
        breaks: "a bridge scope enclosing a function outranks an endpoint scope naming it",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::the_most_specific_scope_governs_across_units",
        },
    },
    Mutation {
        id: 1566,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                            Some((current, _)) if current.strictly_contains(named, &hierarchy) => {\n                                enclosing = Some((named, candidate));\n                            }",
        replace: "                            Some((current, _)) if current.strictly_contains(named, &hierarchy) => {}",
        breaks: "an outer bridge scope outranks an inner one enclosing the same function",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::the_most_specific_scope_governs_across_units",
        },
    },
    Mutation {
        id: 1567,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    catch_all.get_or_insert(candidate);\n                }\n            }",
        replace: "                    catch_all.get_or_insert(candidate);\n                }\n                return catch_all;\n            }",
        breaks: "a catch-all unit listed before a named scope claims the function that scope names",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::the_catch_all_unit_governs_only_functions_no_scope_names",
        },
    },
    Mutation {
        id: 1568,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    .walk(segment, &self.bridge)",
        replace: "                    .walk(segment, &|at| (self.bridge)(at).and(None))",
        breaks: "reserved regions ignore the topology, so a region named through a bridge is never identity-mapped",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_reserved_region_follows_scopes_resolved_through_the_topology",
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
        id: 1573,
        file: "libs/memory-map/src/lib.rs",
        find: "        base <= limit && end.is_some_and(|end| self.allows_device_mapping(start, end))",
        replace: "        base <= limit && end.is_some_and(|end| !self.covers(start, end))",
        breaks: "a reserved DMA region is refused wherever the boot map describes it, device holes included",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::a_reserved_region_inside_a_device_hole_is_admitted",
        },
    },
    Mutation {
        id: 1574,
        file: "libs/memory-map/src/lib.rs",
        find: "        let start = base - base % PAGE_SIZE;",
        replace: "        let start = base.next_multiple_of(PAGE_SIZE);",
        breaks: "a reserved DMA region's first page is rounded up past the kernel memory it starts in",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::a_reserved_region_one_byte_into_kernel_memory_is_refused",
        },
    },
    Mutation {
        id: 1575,
        file: "libs/memory-map/src/lib.rs",
        find: "        let end = limit\n            .checked_add(1)",
        replace: "        let end = limit\n            .checked_add(0)",
        breaks: "a reserved DMA region's last byte is read as its exclusive end",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::a_reserved_region_one_byte_into_kernel_memory_is_refused",
        },
    },
    Mutation {
        id: 1576,
        file: "libs/memory-map/src/lib.rs",
        find: "        base <= limit && end.is_some_and(",
        replace: "        end.is_some_and(",
        breaks: "an empty or inverted reserved DMA region is admitted as the page it rounds to",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::an_empty_or_inverted_reserved_region_is_refused",
        },
    },
    Mutation {
        id: 1577,
        file: "libs/memory-map/src/lib.rs",
        find: "        let start = base - base % PAGE_SIZE;",
        replace: "        let start = base;",
        breaks: "a reserved DMA region may share its first page with kernel memory",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::a_reserved_region_sharing_a_page_with_kernel_memory_is_refused",
        },
    },
    Mutation {
        id: 1578,
        file: "libs/memory-map/src/lib.rs",
        find: "            .checked_add(1)\n            .and_then(|end| end.checked_next_multiple_of(PAGE_SIZE));",
        replace: "            .checked_add(1);",
        breaks: "a reserved DMA region may share its last page with kernel memory",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::a_reserved_region_sharing_a_page_with_kernel_memory_is_refused",
        },
    },
    Mutation {
        id: 1594,
        file: "libs/stack-layout/src/lib.rs",
        find: "            capacity: self.stack_len,",
        replace: "            capacity: 16 * 1024,",
        breaks: "stack usage reports a fixed 16 KiB capacity whatever the slot holds",
        reddens: Caught::HostTest {
            package: "stack-layout",
            name: "usage_splits_a_slot_at_every_pointer_inside_it_and_refuses_the_rest",
        },
    },
    Mutation {
        id: 1595,
        file: "libs/stack-layout/src/lib.rs",
        find: "        let end = limit - limit % WORD;",
        replace: "        let end = limit - limit % WORD + WORD;",
        breaks: "a paintable span reaches one word into the clearance it must leave",
        reddens: Caught::HostTest {
            package: "stack-layout",
            name: "a_span_below_a_pointer_starts_at_the_base_and_stops_a_clearance_short",
        },
    },
    Mutation {
        id: 1603,
        file: "libs/vtd/src/lib.rs",
        find: "        if cap.sagaw_supports(5) {",
        replace: "        if cap.sagaw_supports(5) && Self::Level5.address_bits() <= cap.max_guest_address_width() {",
        breaks: "a supported table is refused when its adjusted width exceeds MGAW",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::supported_table_depth_is_independent_of_guest_address_limit",
        },
    },
    Mutation {
        id: 1604,
        file: "libs/vtd/src/lib.rs",
        find: "        self.covers(address) && (bits == 64 || address >> bits == 0)",
        replace: "        let _ = bits;\n        self.covers(address)",
        breaks: "MGAW no longer bounds an address admitted by the table",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::effective_coverage_obeys_both_table_and_guest_limits",
        },
    },
    Mutation {
        id: 1605,
        file: "libs/vtd/src/lib.rs",
        find: "        self.covers(address) && (bits == 64 || address >> bits == 0)",
        replace: "        bits == 64 || address >> bits == 0",
        breaks: "an address beyond the table's depth is admitted by a wider MGAW",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::effective_coverage_obeys_both_table_and_guest_limits",
        },
    },
    Mutation {
        id: 1636,
        file: "libs/acpi-tables/src/lib.rs",
        find: "        if units > MAX_DRHD_UNITS {",
        replace: "        if false {",
        breaks: "hostile firmware can drive the allocation-free DRHD duplicate scan past its work ceiling",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::excessive_and_ambiguous_remapping_authority_fails_closed",
        },
    },
    Mutation {
        id: 1637,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                            endpoint_ambiguous = true;",
        replace: "                            endpoint_ambiguous = false;",
        breaks: "two endpoint-scoped units can make table order choose DMA authority",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::excessive_and_ambiguous_remapping_authority_fails_closed",
        },
    },
    Mutation {
        id: 1638,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                            Some(_) => {\n                                enclosing_ambiguous = true;\n                            }",
        replace: "                            Some(_) => {\n                                enclosing_ambiguous = false;\n                            }",
        breaks: "two equal-depth bridge scopes can make table order choose DMA authority",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::excessive_and_ambiguous_remapping_authority_fails_closed",
        },
    },
    Mutation {
        id: 1639,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    catch_all_ambiguous = true;",
        replace: "                    catch_all_ambiguous = false;",
        breaks: "two catch-all units can make table order choose DMA authority",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::excessive_and_ambiguous_remapping_authority_fails_closed",
        },
    },
    Mutation {
        id: 1645,
        file: "libs/x86-tables/src/return_state.rs",
        find: "        if self.code_selector != user_code.bits() as u64 {",
        replace: "        if self.code_selector == user_code.bits() as u64 {",
        breaks: "an ordinary ring-three code selector is rejected at the return boundary",
        reddens: Caught::HostTest {
            package: "x86-tables",
            name: "return_state::tests::an_ordinary_user_return_is_accepted",
        },
    },
    Mutation {
        id: 1646,
        file: "libs/x86-tables/src/return_state.rs",
        find: "        if self.instruction_pointer >= user_end {",
        replace: "        if self.instruction_pointer > user_end {",
        breaks: "the first address outside the user half is admitted as a return instruction",
        reddens: Caught::HostTest {
            package: "x86-tables",
            name: "return_state::tests::both_return_addresses_stay_in_the_low_user_half",
        },
    },
    Mutation {
        id: 1647,
        file: "libs/x86-tables/src/return_state.rs",
        find: "        if self.stack_selector != user_data.bits() as u64 {",
        replace: "        if self.stack_selector == user_data.bits() as u64 {",
        breaks: "the installed user stack selector is rejected while a foreign one is admitted",
        reddens: Caught::HostTest {
            package: "x86-tables",
            name: "return_state::tests::both_user_selectors_are_exact",
        },
    },
    Mutation {
        id: 1648,
        file: "libs/x86-tables/src/return_state.rs",
        find: "    | (1 << 21);",
        replace: "    | (1 << 21)\n    | (1 << 14);",
        breaks: "nested-task state is admitted to a 64-bit ring-three return",
        reddens: Caught::HostTest {
            package: "x86-tables",
            name: "return_state::tests::user_flags_are_an_allowlist_with_required_state",
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
        id: 1666,
        file: "libs/acpi-tables/src/lib.rs",
        find: "    if requesters[..*count].contains(&Some(requester)) {\n        return Ok(());\n    }",
        replace: "    let _duplicate_requester = requesters[..*count].contains(&Some(requester));",
        breaks: "duplicate endpoint declarations consume distinct requester owners",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::an_endpoint_inventory_owns_each_distinct_endpoint_on_the_unit",
        },
    },
    Mutation {
        id: 1667,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                        for bus in\n                            scope_hierarchy.buses.secondary..=scope_hierarchy.buses.subordinate\n                        {",
        replace: "                        for bus in\n                            scope_hierarchy.buses.secondary..scope_hierarchy.buses.secondary\n                        {",
        breaks: "a sub-hierarchy requester scan visits none of the buses behind its bridge",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::requester_inventory_expands_hierarchies_catch_all_and_refuses_overflow",
        },
    },
    Mutation {
        id: 1668,
        file: "libs/acpi-tables/src/lib.rs",
        find: "        if self.sole_complete_unit_register_base(selected.segment()) == Some(unit.register_base) {",
        replace: "        if unit.coverage == Coverage::CatchAll {",
        breaks: "a sole catch-all unit with a more-specific scope does not inventory the complete segment",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::requester_inventory_expands_hierarchies_catch_all_and_refuses_overflow",
        },
    },
    Mutation {
        id: 1669,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                        add_live_requester(\n                            self,\n                            &bridge,\n                            &hierarchy,\n                            register_base,\n                            scope_hierarchy.root,\n                            &mut requesters,\n                            &mut count,\n                        )?;",
        replace: "                        let _unowned_hierarchy_root = scope_hierarchy.root;",
        breaks: "the bridge named by a sub-hierarchy is omitted from its unit owner",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_selected_requester_under_a_hierarchy_is_inventoried_with_its_bridge",
        },
    },
    Mutation {
        id: 1670,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    DeviceScopeKind::AcpiNamespaceDevice | DeviceScopeKind::Other { .. } => {\n                        return Err(RequesterInventoryError::UnsupportedScope);\n                    }",
        replace: "                    DeviceScopeKind::AcpiNamespaceDevice | DeviceScopeKind::Other { .. } => {}",
        breaks: "an unfenceable non-PCI or unknown requester scope is silently ignored",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::requester_inventory_refuses_non_pci_and_unknown_requester_scopes",
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
        id: 1673,
        file: "libs/vtd/src/lib.rs",
        find: "        self.write_draining() && self.read_draining()",
        replace: "        self.write_draining() || self.read_draining()",
        breaks: "one DMA drain direction is treated as complete transition draining",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::dma_draining_requires_both_independent_capability_bits",
        },
    },
    Mutation {
        id: 1676,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                            Some(_) => {\n                                enclosing_ambiguous = true;\n                            }",
        replace: "                            Some(_) => enclosing = Some((named, candidate)),",
        breaks: "overlapping sibling hierarchies are ordered instead of refused as ambiguous",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::overlapping_sibling_hierarchies_are_ambiguous_not_numerically_nested",
        },
    },
    Mutation {
        id: 1677,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    if requester_scopes > MAX_REQUESTER_SCOPES {\n                        return Err(RequesterInventoryError::TooManyRequesterScopes {\n                            maximum: MAX_REQUESTER_SCOPES,\n                        });\n                    }",
        replace: "                    let _unbounded_requester_scope_work = requester_scopes;",
        breaks: "duplicate hierarchy scopes amplify a bounded requester result into unbounded PCI probing",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::requester_scope_work_is_bounded_before_duplicate_hierarchy_scans",
        },
    },
    Mutation {
        id: 1678,
        file: "libs/vtd/src/lib.rs",
        find: "    const DRAIN_READS: u64 = 1 << 49;",
        replace: "    const DRAIN_READS: u64 = 1 << 50;",
        breaks: "the pre-root DMA drain requests a reserved bit instead of draining reads",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::iotlb_command_words_and_readback",
        },
    },
    Mutation {
        id: 1680,
        file: "libs/acpi-tables/src/lib.rs",
        find: "    if !path.len().is_multiple_of(2)\n        || matches!(",
        replace: "    if matches!(",
        breaks: "a trailing byte in a DMAR device-scope path is silently discarded",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::odd_and_empty_pci_paths_in_every_scoped_dmar_structure_are_refused",
        },
    },
    Mutation {
        id: 1681,
        file: "libs/acpi-tables/src/lib.rs",
        find: "        ) && path.is_empty()\n    {",
        replace: "        ) && false\n    {",
        breaks: "an endpoint or hierarchy device scope is accepted without a PCI path",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::odd_and_empty_pci_paths_in_every_scoped_dmar_structure_are_refused",
        },
    },
    Mutation {
        id: 1682,
        file: "libs/acpi-tables/src/lib.rs",
        find: "    path.chunks_exact(2).all(|hop| hop[0] < 32 && hop[1] < 8)",
        replace: "    path.chunks_exact(2).all(|_hop| true)",
        breaks: "a DMAR path hop can encode a nonexistent PCI device or function number",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::device_scope_iteration_defensively_refuses_every_bad_pci_hop",
        },
    },
    Mutation {
        id: 1683,
        file: "libs/acpi-tables/src/lib.rs",
        find: "        if !device_scope_is_well_formed(entry) {\n            self.bytes = &[];\n            return None;\n        }",
        replace: "        let _unchecked_device_scope = entry;",
        breaks: "the defensive device-scope iterator resumes through malformed path bytes",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::device_scope_iteration_defensively_refuses_every_bad_pci_hop",
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
        id: 1688,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    || (scope.buses.secondary..=scope.buses.subordinate).contains(&bdf.bus())\n                        && contains(scope.root, bdf)",
        replace: "                    || (scope.buses.secondary..=scope.buses.subordinate).contains(&bdf.bus())",
        breaks: "a numeric bridge bus interval is accepted as DMAR hierarchy ancestry",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_numeric_bus_range_without_ancestry_names_no_requester_or_reserved_region",
        },
    },
    Mutation {
        id: 1691,
        file: "libs/acpi-tables/src/lib.rs",
        find: "        let mut hops = self.path();",
        replace: "        if self.path.len() > 16 {\n            return None;\n        }\n        let mut hops = self.path();",
        breaks: "a valid nine-hop endpoint disappears and lets another unit's outer hierarchy govern it",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_long_endpoint_path_still_outranks_another_units_outer_hierarchy",
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
        id: 1703,
        file: "libs/acpi-tables/src/lib.rs",
        find: "        self.body[Sdt::LEN + 1] & 1 != 0",
        replace: "        self.body[Sdt::LEN + 1] & (1 << 1) != 0",
        breaks: "the DMAR interrupt-remapping flag is decoded from x2APIC opt-out",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::dmar_platform_flags_decode_interrupt_remapping_and_x2apic_opt_out",
        },
    },
    Mutation {
        id: 1704,
        file: "libs/acpi-tables/src/lib.rs",
        find: "        self.body[Sdt::LEN + 1] & (1 << 1) != 0",
        replace: "        self.body[Sdt::LEN + 1] & 1 != 0",
        breaks: "the DMAR x2APIC opt-out flag is decoded from interrupt-remapping support",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::dmar_platform_flags_decode_interrupt_remapping_and_x2apic_opt_out",
        },
    },
    Mutation {
        id: 1705,
        file: "libs/vtd/src/lib.rs",
        find: "    pub const fn interrupt_remapping(self) -> bool {\n        (self.0 & Self::IR) != 0\n    }",
        replace: "    pub const fn interrupt_remapping(self) -> bool {\n        (self.0 & Self::EIM) != 0\n    }",
        breaks: "VT-d interrupt-remapping support is decoded from the EIM bit",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::extended_capability_decodes_each_field",
        },
    },
    Mutation {
        id: 1706,
        file: "libs/vtd/src/lib.rs",
        find: "    pub const fn extended_interrupt_mode(self) -> bool {\n        (self.0 & Self::EIM) != 0\n    }",
        replace: "    pub const fn extended_interrupt_mode(self) -> bool {\n        (self.0 & Self::IR) != 0\n    }",
        breaks: "VT-d extended interrupt mode is decoded from basic IR support",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::extended_capability_decodes_each_field",
        },
    },
    Mutation {
        id: 1710,
        file: "libs/vtd/src/lib.rs",
        find: "    Some((address & REMAPPING_ADDRESS_MASK) | (1 << 11))",
        replace: "    Some(address & REMAPPING_ADDRESS_MASK)",
        breaks: "the kernel interrupt-remapping table is published without extended interrupt mode",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::interrupt_table_and_queue_addresses_are_exact_and_aligned",
        },
    },
    Mutation {
        id: 1711,
        file: "libs/vtd/src/lib.rs",
        find: "    pub const fn global_interrupt_entry_cache() -> Self {\n        Self { lo: 4, hi: 0 }\n    }",
        replace: "    pub const fn global_interrupt_entry_cache() -> Self {\n        Self {\n            lo: 4 | (1 << 4),\n            hi: 0,\n        }\n    }",
        breaks: "interrupt-entry cache invalidation sets index-selective granularity instead of flushing every entry",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::queued_interrupt_cache_invalidation_descriptors_match_the_spec",
        },
    },
    Mutation {
        id: 1712,
        file: "libs/vtd/src/lib.rs",
        find: "            lo: 5 | (1 << 5) | ((data as u64) << 32),",
        replace: "            lo: 5 | ((data as u64) << 32),",
        breaks: "queued invalidation waits without writing the completion status",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::queued_interrupt_cache_invalidation_descriptors_match_the_spec",
        },
    },
    Mutation {
        id: 1737,
        file: "libs/vtd/src/lib.rs",
        find: "pub const FECTL: u64 = 0x38;",
        replace: "pub const FECTL: u64 = 0x3C;",
        breaks: "the fault-event interrupt mask is written through the wrong register offset",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::register_offsets_are_the_spec_byte_offsets",
        },
    },
    Mutation {
        id: 1738,
        file: "libs/vtd/src/lib.rs",
        find: "        control & MASK != 0",
        replace: "        control & PENDING != 0",
        breaks: "fault-event pending state is mistaken for the interrupt mask",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::fault_event_control_masks_the_non_remapped_message_source",
        },
    },
    Mutation {
        id: 1759,
        file: "libs/x86-apic/src/pending.rs",
        find: "    registers[raw / 32] & (1 << (raw % 32)) != 0",
        replace: "    highest(registers) == Some(vector.get())",
        breaks: "a named pending vector is hidden by unrelated higher-priority work",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "pending::tests::a_named_pending_vector_is_visible_beneath_higher_priority_work",
        },
    },
    Mutation {
        id: 1760,
        file: "libs/x86-apic/src/version.rs",
        find: "        (self.0 >> MAXIMUM_LVT_ENTRY_SHIFT) as u8",
        replace: "        self.number()",
        breaks: "the APIC source inventory is decoded from the version byte instead of its independent maximum-LVT field",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "version::tests::the_lvt_limit_is_independent_of_the_version_number",
        },
    },
    Mutation {
        id: 1761,
        file: "libs/x86-apic/src/register.rs",
        find: "    LVT_TIMER,\n    LVT_THERMAL,\n    LVT_PERFORMANCE,",
        replace: "    LVT_TIMER,\n    LVT_TIMER,\n    LVT_PERFORMANCE,",
        breaks: "the thermal source is absent from the version-ordered local-vector inventory",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "register::tests::the_local_vector_entries_sit_where_the_manual_puts_them",
        },
    },
    Mutation {
        id: 1780,
        file: "libs/vtd/src/lib.rs",
        find: "        matches!(self.major(), 1..=5)",
        replace: "        matches!(self.major(), 1..=6)",
        breaks: "VT-d version six is incorrectly allowed to use ignored register invalidations",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::version_bounds_the_register_invalidation_interface",
        },
    },
    Mutation {
        id: 1803,
        file: "libs/acpi-tables/src/lib.rs",
        find: "        (units.next().is_none() && unit_segment == segment).then_some(register_base)",
        replace: "        units.next().is_none().then_some(register_base)",
        breaks: "the early owner accepts the sole remapping unit from another PCI segment",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_single_catch_all_drhd_without_rmrr_exposes_its_register_base",
        },
    },
    Mutation {
        id: 1806,
        file: "libs/vtd/src/lib.rs",
        find: "status & (gsts::TES | gsts::QIES | gsts::IRES | gsts::CFIS) == 0",
        replace: "status & (gsts::QIES | gsts::IRES | gsts::CFIS) == 0",
        breaks: "takeover attempts a live non-identical root replacement while translation is active",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::takeover_requires_every_firmware_facility_to_be_inactive",
        },
    },
    Mutation {
        id: 1807,
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    DmarStructure::Drhd {\n                        include_pci_all: true,\n                        register_base: candidate,\n                        ..\n                    } if candidate == register_base",
        replace: "                    DmarStructure::Drhd {\n                        include_pci_all: _,\n                        register_base: candidate,\n                        ..\n                    } if candidate == register_base",
        breaks: "a scoped unit is mistaken for a whole-segment DMA boundary",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_single_catch_all_drhd_without_rmrr_exposes_its_register_base",
        },
    },
    Mutation {
        id: 1808,
        file: "libs/acpi-tables/src/lib.rs",
        find: "        if self.entries().any(\n            |structure| matches!(structure, DmarStructure::Rmrr { segment: reserved, .. } if reserved == segment),\n        ) {\n            return None;\n        }",
        replace: "        let _reserved_regions_are_ignored = segment;",
        breaks: "early takeover revokes a firmware-reserved DMA mapping before refusing the machine",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_single_catch_all_drhd_without_rmrr_exposes_its_register_base",
        },
    },
    Mutation {
        id: 1821,
        file: "libs/acpi-tables/src/lib.rs",
        find: "/// Why one DRHD cannot become a complete fixed requester inventory.",
        replace: "/// The retired policy that refused a catch-all-selected requester.\npub const fn selected_uses_catch_all() -> RequesterInventoryError {\n    RequesterInventoryError::IncludesAllPci\n}\n\n/// Why one DRHD cannot become a complete fixed requester inventory.",
        breaks: "the retired catch-all selection refusal returns as dead public API",
        reddens: Caught::ApiAudit("fn selected_uses_catch_all"),
    },
    Mutation {
        id: 1822,
        file: "libs/acpi-tables/src/lib.rs",
        find: "        if flags != Self::REQUIRED_FLAGS {",
        replace: "        if flags & Self::REQUIRED_FLAGS == 0 {",
        breaks: "an incomplete DMA handoff contract is accepted when any one platform flag remains",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::dma_handoff_requires_every_exact_platform_property",
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
        id: 1838,
        file: "libs/x86-cpuid/src/lib.rs",
        find: "            ExecutionEnvironmentKind::Vmware => Some(DmaHandoffPlatform::Vmware),",
        replace: "            ExecutionEnvironmentKind::Vmware => None,",
        breaks: "the qualified VMware CPU root can no longer authenticate its DMA handoff",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::only_exact_qualified_hypervisor_leaves_authenticate_dma_handoff",
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
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
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
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
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
        id: 2049,
        file: "libs/x86-paging/src/kpti.rs",
        find: "    if expected.is_empty() || expected.len() != observed.len() {\n        return false;\n    }",
        replace: "    if expected.is_empty() || expected.len() == observed.len() {\n        return false;\n    }",
        breaks: "the exact stripped-root enclave refuses its complete expected page set",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "kpti::tests::a_kpti_user_root_contains_only_the_entry_enclave",
        },
    },
    Mutation {
        id: 2050,
        file: "libs/x86-paging/src/kpti.rs",
        find: "    full.table_identity_matches(stripped)\n        && full.flags().difference(permitted).bits() == 0",
        replace: "    !full.table_identity_matches(stripped)\n        && full.flags().difference(permitted).bits() == 0",
        breaks: "a paired root rejects its one identical shared lower-half subtree",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "kpti::tests::a_kpti_pair_shares_one_identical_user_subtree",
        },
    },
    Mutation {
        id: 2051,
        file: "libs/x86-paging/src/kpti.rs",
        find: "        if self.root_frames != 2 {\n            return None;\n        }",
        replace: "        if self.root_frames == 2 {\n            return None;\n        }",
        breaks: "incomplete root construction can publish while a complete pair cannot",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "kpti::tests::a_kpti_pair_refusal_returns_every_frame",
        },
    },
    Mutation {
        id: 2052,
        file: "libs/x86-paging/src/kpti.rs",
        find: "        if returned_frames == 0 {\n            return Ok(PairCleanupState::Retained(self));\n        }",
        replace: "        if returned_frames == 1 {\n            return Ok(PairCleanupState::Retained(self));\n        }",
        breaks: "a zero-return cleanup refusal no longer retains complete pair authority",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "kpti::tests::a_kpti_pair_cleanup_retains_complete_authority",
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
        id: 2289,
        file: "libs/x86-tables/src/gdt.rs",
        find: "            | bit::RING_THREE\n            | bit::ACCESSED,\n    );",
        replace: "            | bit::RING_THREE,\n    );",
        breaks: "the first iretq to ring 3 must write the user code descriptor's accessed bit",
        reddens: Caught::HostTest {
            package: "x86-tables",
            name: "gdt::tests::every_code_and_data_descriptor_is_already_accessed",
        },
    },
    Mutation {
        id: 2290,
        file: "libs/x86-tables/src/gdt.rs",
        find: "bit::RING_THREE | bit::ACCESSED);",
        replace: "bit::RING_THREE);",
        breaks: "the first iretq to ring 3 must write the user data descriptor's accessed bit",
        reddens: Caught::HostTest {
            package: "x86-tables",
            name: "gdt::tests::every_code_and_data_descriptor_is_already_accessed",
        },
    },
    Mutation {
        id: 2291,
        file: "libs/x86-tables/src/gdt.rs",
        find: "            | bit::LONG_MODE\n            | bit::ACCESSED,\n    );\n\n    /// Ring 0 data",
        replace: "            | bit::LONG_MODE,\n    );\n\n    /// Ring 0 data",
        breaks: "loading the kernel code selector writes the descriptor table",
        reddens: Caught::HostTest {
            package: "x86-tables",
            name: "gdt::tests::every_code_and_data_descriptor_is_already_accessed",
        },
    },
    Mutation {
        id: 2292,
        file: "libs/x86-tables/src/gdt.rs",
        find: "Self(bit::PRESENT | bit::CODE_OR_DATA | bit::ACCESS | bit::ACCESSED);",
        replace: "Self(bit::PRESENT | bit::CODE_OR_DATA | bit::ACCESS);",
        breaks: "loading the kernel data selector writes the descriptor table",
        reddens: Caught::HostTest {
            package: "x86-tables",
            name: "gdt::tests::every_code_and_data_descriptor_is_already_accessed",
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
            name: "dma_contracts::isa_dma_adapter_clears_masks_and_rechecks_both_controllers",
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
        id: 2426,
        file: "libs/frame-space/src/lib.rs",
        find: "    if !address.is_multiple_of(FRAME_SIZE) {",
        replace: "    if false && !address.is_multiple_of(FRAME_SIZE) {",
        breaks: "an unaligned physical address aliases an aligned frame identity",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::frame_identity_is_exact_at_zero_high_and_misaligned_boundaries",
        },
    },
    Mutation {
        id: 2427,
        file: "libs/frame-space/src/lib.rs",
        find: "    NonZeroU64::new(address / FRAME_SIZE + 1)",
        replace: "    NonZeroU64::new(address / FRAME_SIZE)",
        breaks: "physical frame zero has no mapping-owner identity",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::frame_identity_is_exact_at_zero_high_and_misaligned_boundaries",
        },
    },
    Mutation {
        id: 2428,
        file: "libs/frame-space/src/lib.rs",
        find: "    NonZeroU64::new(address / FRAME_SIZE + 1)",
        replace: "    NonZeroU64::new(((address / FRAME_SIZE) as u32) as u64 + 1)",
        breaks: "high physical frames collide after their identity is truncated to thirty-two bits",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::frame_identity_is_exact_at_zero_high_and_misaligned_boundaries",
        },
    },
    Mutation {
        id: 2454,
        file: "libs/register-bits/src/lib.rs",
        find: "pub const fn low_u32(value: u64) -> u32 {\n    value as u32\n}",
        replace: "pub const fn low_u32(value: u64) -> u32 {\n    (value | value >> 32) as u32\n}",
        breaks: "a reserved high bit set by the hardware leaks into a 32-bit register read",
        reddens: Caught::HostTest {
            package: "register-bits",
            name: "tests::a_32_bit_read_ignores_the_reserved_high_half",
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
    Mutation {
        id: 2457,
        file: "libs/vtd/src/lib.rs",
        find: "const REASON_SHIFT: u32 = 32;",
        replace: "const REASON_SHIFT: u32 = 40;",
        breaks: "a fault record reports its PASID as the fault reason",
        reddens: Caught::HostTest {
            package: "vtd",
            name: "tests::a_recorded_fault_decodes_its_source_reason_and_address",
        },
    },
    Mutation {
        id: 2458,
        file: "libs/memory-map/src/lib.rs",
        find: "let overlaps = region.len != 0 && start < region.end() && region.base < end;",
        replace: "let overlaps = start < region.end() && region.base < end;",
        breaks: "an empty region blocks device mappings around its base",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::a_zero_length_region_does_not_occupy_its_address",
        },
    },
    Mutation {
        id: 2459,
        file: "libs/memory-map/src/lib.rs",
        find: "        if start > end {\n            return false;\n        }",
        replace: "        if false {\n            return false;\n        }",
        breaks: "an inverted span reads as covered memory",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::an_inverted_span_is_not_covered",
        },
    },
    Mutation {
        id: 2460,
        file: "libs/fat32/src/lib.rs",
        find: ".ok_or(FatError::Malformed)?;",
        replace: ".ok_or(FatError::BadCluster(0))?;",
        breaks: "a wrapped sector address is reported as cluster 0",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_partition_base_that_wraps_is_malformed",
        },
    },
];
