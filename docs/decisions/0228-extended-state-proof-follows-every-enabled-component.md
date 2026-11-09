# D-228 Extended-state proof follows every enabled component

Status: Accepted (2026-08-29), advances PW-305 to VERIFY

**Context.** The eager switch saves the `XCR0` mask x87 + SSE + AVX, but its
only restored-value witness wrote the low quadword of `xmm0`. That observes SSE
inside the legacy area. It says nothing about the x87 stack beside it or AVX's
upper halves in component 2, so either component could be omitted while the
check and D-108's claim remained green. D-144's sentinel proves the selected
form stays inside its buffer; it deliberately does not prove the form wrote
everything it claims.

**Decision.** One narrow `xstate::Probe` is the complete machine witness for
the mask this kernel permits. It carries an exactly representable x87 integer,
the low quadword of `xmm0`, and the high quadword of `ymm0`. Its write pushes
the x87 value, writes the SSE lane and inserts the AVX lane; its read extracts
the lanes and pops the x87 value. The same two-thread switch experiment gives
both threads three different values and requires all three to return intact.
The x87 write resets that stack before pushing: the first break attempt merely
pushed both threads' values, and their LIFO execution order made an omitted
x87 save look correct. With the reset, each write replaces the live witness
instead of leaving the other thread's value underneath it. The wrappers refuse
unless extended state is live and all three component bits are enabled. They
remain the only target code which uses floating-point/vector instructions,
under the existing `+soft-float,-sse` contract.

One named operation mask feeds both save forms and restore. Persistent
mutations clear x87, SSE and AVX from that mask independently, so each claimed
component must make the same machine check red. The existing mutation which
skips the complete save/restore path remains the fourth witness.

**Form coverage.** The ordinary QEMU `-cpu max` boot executes standard
`XSAVE`; the current TCG explicitly warns that it cannot provide an opt-in
`XSAVEC` bit. A current VMware self-test must therefore execute the same
three-component check through production `XSAVEC`. Historical compacted boot
evidence predates this witness and is not credited to it. PW-305 becomes
`VERIFY`, owned by PW-003's current VMware round rather than by a fake CPUID
configuration.

**Alternatives.** Inspecting saved bytes was rejected because x87/SSE legacy
encoding and compacted AVX placement would make the check another parser of
the implementation. Checking three registers without a switch was rejected
because it proves only that the probe instructions agree with themselves.
Enabling AVX-512, AMX or another component solely to test it was rejected:
every enabled bit costs per-thread memory and D-82 admits only components with
a caller. Forcing `+xsavec` under TCG was rejected after the emulator reported
that it cannot implement the requested feature.

**Consequences.** Release QEMU passed all 213 checks through standard `XSAVE`.
Clearing x87, SSE or AVX independently then reddened only `every enabled state
component survives a switch`, each at 212/213; mutations 259 through 261 retain
those exact defects. The initial push-only x87 witness staying green under its
omission is itself the D-28 result which required the reset. The compacted half
remains one specific hardware observation, not open-ended implementation work;
once the current VMware image passes the same named check, PW-305 is `DONE`.
No state beyond the already enabled mask is added. The reviewed target wrapper
adds ten lines to the unsafe audit surface, now exactly 380 against its
unchanged 380-line ceiling, and advances only `xstate.rs`'s pinned fingerprint.

**Current disposition (2026-08-31).** The implementation finding is resolved,
so PW-305 is now `REQUIRED` rather than `VERIFY`: only PW-003's fresh
compacted-`XSAVEC` VMware observation remains.
