# D-252 A recoverable fault carries its one permitted destination

Status: Accepted, 2026-08-30. Amends D-120 and D-186.

**Context.** Validating a page fault and then passing a raw resume address to
the trap-frame writer left a second authority door: any safe in-module caller
could redirect `iretq`, and a helper could move the fixup construction outside
the validated adapter while the original checks stayed green.

**Decision.** User-copy and write-protection recovery each use an opaque
`pub(super)` target with a private field. Its sole private constructor names the
exact linker fixup; its observer returns only that field. The interrupt adapter
accepts the target rather than `u64`, first proves the frame is still the live
innermost frame, and performs the sole instruction-pointer write. Recursive
source contracts inventory nested modules, associated methods, constructors,
fixup references and writers.

**Evidence.** Moving the fixup call into a helper, adding a nested raw
constructor, changing the linker symbol, changing the private-field observer
and hiding a writer in a nested module each failed its exact contract after the
collector was repaired. The restored five-contract suite and the real
user-copy recovery machine check pass.
