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
        id: 429,
        file: "libs/timebase/src/lib.rs",
        find: "        let carried = advanced - (whole << self.shift);",
        replace: "        let carried = 0;",
        breaks: "each clock update discards its fractional nanosecond carry",
        reddens: Caught::HostTest {
            package: "timebase",
            name: "tests::published::a_thousand_updates_do_not_drift_from_one_conversion",
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
            name: "tests::rights::attenuation_only_narrows",
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
];
