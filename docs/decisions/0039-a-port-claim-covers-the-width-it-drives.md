# D-39 A port claim covers the width it drives

Status: Accepted, 2026-08-22.

**Context.** An adversarial audit of Phase 2 found `wost::arch::x86_64::port`
recording one bit per claim whatever the access width. `port::claim::<u32>`
authorises an `out dx, eax`, which drives four consecutive ports; the registry
believed it owned one. `kernel/src/arch/x86_64/debug_exit.rs` is a live
instance: it claims `Port<u32>` at `0xF4` and the harness gives QEMU
`iosize=0x04`, so the kernel drives `0xF4..0xF8` against a one-port claim.
`PortBlock` had the mirror of the same hole — `port_for` asserted
`offset < N`, so a `u32` access at `N - 1` ran three ports past the block into
whatever answered next.

Nothing collided in practice; every `PortBlock` caller happens to use `u8`.
But `005-code-conventions.md` states that what `wost` owes its callers is that
every abstraction make the unsafe operation underneath it *checkable* — and
names `port::claim` as the worked example, "so exclusivity is verified rather
than promised". It was promised.

**Options.** (a) Leave it and document the caveat; (b) widen the claim and the
bounds check in place; (c) widen them, and move the bookkeeping somewhere it
can be tested.

**Decision.** (c). `size_of::<T>()` is the number of ports an access drives —
which the sealed `PortValue` trait makes true by construction, since its only
implementors are the primitives of exactly those widths. `claim` reserves that
many; `PortBlock::read`/`write` check `offset + size_of::<T>() <= N`. The
bitmap itself moved to a new `libs/port-space` crate, on D-30's precedent: it
is arithmetic over bits with no hardware behind it, so it belongs where
`cargo test` can reach it.

`Port::new` and `PortBlock::new` became private in the same change. They were
`pub unsafe`, which left a documented way to construct a handle the registry
knew nothing about; `claim` and `claim_block` are now the only doors.

**Rationale.** The width is not a detail a caller should have to remember —
it is already in the type. Extracting the bitmap bought thirteen host tests
including a sweep of every way two spans can meet and a pass over all 65536
ports, none of which a `no_std` cross-compiled module could have carried.

**Consequences.** One on-machine check joins them (`a claim covers the width
it drives`): a 32-bit claim two ports below COM1 must be refused, and a byte
claim at the same port must then succeed — which proves both that the port
was free, so the refusal was about the width, and that the refused claim
recorded nothing. Broken once by making `reserve` ignore its length, and the
boot went red. Phase 3's device mappings are to inherit this discipline, not
the old one: `007-memory.md` asks for "a claim discipline like `port::claim`'s"
and that now means covering the span an access reaches.
