# D-12 Security baseline from day one

Status: Accepted, 2026-07-18.

**Decision.** W^X for every mapping, NX, SMEP + SMAP + UMIP in CR4
during early boot, KASLR (via Limine) once addresses aren't hardcoded,
guard pages under all stacks, zero-on-allocate. Later: Intel CET shadow
stacks, signed/measured boot.

**Rationale.** 2026 table stakes; retrofitting mitigations is how
systems end up with exceptions. Costs almost nothing at this codebase
size; costs everything later.

**Consequences.** Phase 2/3 code must set this up before conveniences
(e.g. no RWX "temporary" mappings, ever).
