# D-158 One ELF reader, and the verifier uses the loader's

Status: Accepted, 2026-08-25.

**Decision.** `xtask` reads the ELF file header and program headers through
`libs/elf64`, the crate the kernel loads programs with, and keeps only the
section table. `xtask::elf::parse` is `elf64::Elf::parse(bytes)` plus a section
walk; `parse_segments` and every transcribed constant and offset are deleted.
The section table stays in `xtask` because a loader never reads one, and moving
it would grow the crate the kernel links for one host caller. `elf64` gains
`SegmentKind::bits`, the inverse of `from_bits`, and `Segment::flags`, the raw
field a verifier reports, both with their caller in the same commit (D-41).

**Rationale.** Two readers of one format with nothing comparing them had
already drifted: D-124 found `xtask`'s copy two validations short, so the build
verifier would have shipped an aarch64 kernel image. The rule this settles is
the same one D-157 stated a level up: a reader that restates the format another
reader already knows passes on the day the two copies disagree. The merge also
makes `xtask` stricter: `elf64` takes the whole 64-byte header before reading
the class byte, checks the program-header table lies inside the image without
overflowing, and refuses `p_phoff` near `u64::MAX`.

**Consequences.**

- `libs/elf64` gains a round-trip test over the `p_type` table, exhaustive
  across the named range and its edges, which also proves no two named kinds
  encode alike.
- The old sync test is renamed for what it proves, that an image for another
  machine or of another shape is refused, and still asserts the two constants,
  because a typo in `elf64` would move both readers together.
- `libs/elf64`'s module doc says the merge happened and what deliberately
  stayed behind.
