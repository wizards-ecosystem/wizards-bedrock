# D-266 Unsafe instruction doors expose only their checked caller scope

Status: Accepted, 2026-08-31. Amends D-41 and D-123; advances PW-119 and PW-133.

**Decision.** Safe instruction adapters accept the narrowest typed operand that
proves the architectural precondition. The raw instruction remains unsafe and
visible only to the sibling architecture module which implements that adapter.
A helper which already consumes a checked type, such as the FS-base writer,
remains safe and private to its sole caller rather than becoming another raw
door. Recursive syntax inventory rejects every unrestricted public unsafe
function anywhere below `wost/src`.

**Rationale.** A safe raw-address `invlpg` door admitted a faulting
noncanonical operand, while CR3, XSAVE and FS-base helpers were visible farther
than any compiled caller. Public unsafe is not safer than private unsafe: it
widens the set of contracts safe wrappers must defend and invites a second
unchecked route.

**Alternatives and current evidence.** Documentation on a public integer was
rejected because safe callers cannot be required to uphold it. `pub(crate)` was
rejected where no crate-wide caller exists. Canonical cutover tests, compiler
graphs and AST contracts cover the four narrowed doors. Separate visibility and
safety-shape mutations cover each one, and an unrelated public `wrmsr` mutation
proves the inventory is global. The current machine and API gates remain in the
register. It does not claim that every safe API is narrow; later usercopy, PCI
and vector-door audits are separate.
