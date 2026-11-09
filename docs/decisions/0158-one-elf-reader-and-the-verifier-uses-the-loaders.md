# D-158 One ELF reader, and the verifier uses the loader's

Status: Accepted (2026-08-25)

**Context.** D-105 named it and left it: `xtask/src/elf.rs` carried its own
transcription of the ELF magic, the class byte, the endianness byte, `PT_LOAD`,
`PF_W`, `PF_X` and every file- and program-header offset, while `libs/elf64` —
the crate the kernel actually loads programs with — carried the same table
again. Two readers of one format in one repository, with nothing comparing
them.

The cost was not hypothetical and had already been paid twice. D-124 added
`e_machine` and the program-header stride checks to `elf64` and found `xtask`'s
copy *two validations short*, so the build verifier would have shipped an
aarch64 kernel image. A test named
`the_two_refusals_libs_elf64_has_are_here_too` existed for no reason except to
hand-maintain the sync — a test whose name concedes the defect.

**Options.** Leave it and keep the sync test. Move the section reader into
`elf64` so one crate reads everything. Or have `xtask` read the header and the
program headers through `elf64` and keep only the section table.

**Decision.** The third. `xtask::elf::parse` is now
`elf64::Elf::parse(bytes)` plus a section walk; `parse_segments` and every
transcribed constant are deleted. The section table stays in `xtask` because a
*loader* never reads one and nothing else in the tree wants it — moving it into
`elf64` would grow the crate the kernel links for the benefit of one host
caller.

Two accessors were added to `elf64` to make it a complete substitute:
`SegmentKind::bits`, the inverse of `from_bits`, and `Segment::flags`, the raw
field a verifier needs in order to report what it objected to. Neither is
speculative — both have their caller in the same commit (D-41).

**Rationale.** The rule this settles is the same one D-157 stated a level up.
There, a check restated the data it was checking; here, a reader restated the
format another reader already knew. Both pass on the day the two copies
disagree, because the copy being read is the copy the author was looking at.
`x86-tables` and `trap-cause` disagreeing about vector 28 is the same story
with a third table.

The merge also makes `xtask` *stricter* for free, which is the tell that it was
behind: `elf64` takes the whole 64-byte file header before it looks at the
class byte, checks that the program-header table lies inside the image without
overflowing, and refuses `p_phoff` near `u64::MAX` — none of which the
verifier's own parser did.

**Consequences.**

- `xtask/src/elf.rs` loses seven constants, `parse_segments`, and every header
  offset except the section table's. `libs/elf64` gains two accessors and a
  round-trip test over the `p_type` table, exhaustive across the named range
  and its edges, which also proves no two named kinds encode alike.
- Every existing ELF test passes unchanged, which is the evidence the two
  readers did agree — and `elf64`'s error messages already contained the
  strings the verifier's tests assert on.
- Two tests were reframed rather than deleted. The sync test is renamed for
  what it now proves — an image for another machine or of another shape is
  refused — and still asserts the two constants, because a typo in `elf64`
  would move both readers together and neither would notice.
- `libs/elf64`'s module doc said the merge had not happened. It now says it
  has, and says what deliberately stayed behind.
