# D-213 Stable rustc resolves the public foundation surface

Status: Accepted, 2026-08-28. Amends D-41 and D-158; closes PW-505.

**Context.** D-41 forbids speculative public API, but the last sweep searched
source text for names. Text cannot distinguish two same-name methods, a real
call from documentation, or a re-export from executable use. Stable rustc does
not apply `dead_code` across a library's public boundary, and rustdoc JSON is
still a nightly interface. The result was an unenforced rule and a public
surface that lint-driven `is_empty` additions could grow without a caller.

D-158 removed the duplicate ELF loader and deliberately left the section walk
in `xtask`, but that walk still read `e_shoff`, `e_shentsize`, `e_shnum` and
`e_shstrndx` from four private numeric offsets. The section-entry and string
interpretation has one host caller and does not belong in the target loader;
the file-header field locations are nevertheless ELF knowledge the loader
already owns.

**Decision.** `cargo xtask api-audit` copies the Cargo inputs, `libs/`, `wost`,
`kernel`, `tools` and `xtask` into a locked workspace under `target/api-audit`
and compiles all four D-212 graphs offline with a private target directory. It
uses `syn` and `proc-macro2` source spans to give every public item declaration
and enum variant in `libs/` and `wost` a unique `#[deprecated]` marker. An
equal-weight antichain schedules those markers so every ordered pair is
separated in at least one compiler round. Stable rustc's resolved diagnostics
then establish calls between audited items and calls from code outside the
inventory; transitive reachability from those external roots is the live
surface.

A public re-export is exposure, not a caller, and is excluded by its resolved
source span. Host graphs compile all targets, so tests and examples are real
roots. Exact source identity keeps same-name items separate, while comments
and rustdoc prose emit no compiler use and therefore cannot keep an API alive.
The audit is an ordered CI stage between clippy and rustdoc.

The inventory deliberately excludes fields. A measured prototype reported
nine apparently dead fields, including fields demonstrably read by pattern
destructuring: stable rustc does not emit a deprecation diagnostic for every
such use. Claiming compiler proof there would be false. Fields remain in the
manual semantic surface review until a stable compiler interface can resolve
them completely; this decision proves item declarations and enum variants
only.

`libs/elf64::SectionTable` now parses and owns the section table's four
file-header values. `xtask` consumes that typed value and retains only the
host-specific section-entry walk and symbol/string checks. No private offset
transcription is described as canonical.

**Rationale.** Compiler name resolution is the authority the text sweep was
trying to approximate. Deprecation diagnostics are stable, carry resolved use
spans, work across the actual feature/target graphs, and need no network tool
or nightly artifact. The antichain reduces 1,466 one-item builds to thirteen
rounds without losing direction: for any caller and callee, one round marks the
callee while leaving the caller unmarked, so suppression inside deprecated
code cannot hide that edge.

Keeping the isolated copy is part of the evidence. Instrumentation never
rewrites the checkout, and an interrupted compiler leaves only replaceable
state below `target`. The same exclusive-lock and private-target properties
that make mutation runs resumable prevent two audits from corrupting one
another or a normal build.

**Alternatives.** A stronger text search was rejected because parsing syntax
still does not resolve names. Adding an external public-API tool was rejected
because it would add a network/toolchain exception and still answer API
compatibility rather than caller reachability. Nightly rustdoc JSON or compiler
internals were rejected because the repository's stable toolchain is a gate,
not a suggestion. Treating re-exports as roots was rejected because it would
preserve an API merely for being public. Moving the complete section reader to
`elf64` was rejected because the target loader has no section-entry consumer.

**Consequences.** The first compiler run inventoried 1,489 non-field items and
found 29 with no resolved caller. The unused surface was deleted: one ACPI
revision accessor, one ELF byte-slice accessor, one FAT source conversion, one
MMIO emptiness query, four NVMe register/configuration constants, one PCI bus
count, one x86-APIC bit constructor, and nineteen `wost` address/count/accessor,
frame/segment, port, xstate and lock operations. Four narrow
`len_without_is_empty` expectations state why their types are constructibly
non-empty instead of manufacturing dead `is_empty` methods to satisfy a style
lint. Removing the unused raw frame writer reduced the pinned `wost` unsafe
surface from 370 to 368 lines without raising its 380-line budget.

After the typed ELF metadata additions, all 1,466 audited declarations and
variants have a compiler-resolved path to a real caller in thirteen rounds.
Re-export-as-use, doc-as-use and wrong-same-name defects each made only
`reexports_docs_and_same_names_are_resolved_by_the_compiler` red. Reading the
section offset from `e_phoff` made only
`the_section_table_fields_are_where_the_file_header_puts_them` red.
Reintroducing the dead MMIO `is_empty` made the complete audit report exactly
`method impl::is_empty`; it also proves the mutation runner's new typed
API-audit baseline and attribution route. All five transformations were
restored and persist, bringing the registry to 195 and the local gate to ten
ordered stages.
