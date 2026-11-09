# D-48 The console scrolls out of RAM

Status: Accepted (2026-08-22)

**Context.** `001-boot.md` has carried a known limitation since Phase 1: the
framebuffer console reads uncached device memory back in order to scroll.
`007-memory.md` scheduled the fix as the heap's first consumer, and
`008-research-notes-2.md` §3.4 argued something stronger — that the back buffer
and the write-combining decision are *one* decision, because WC makes writes
fast and reads catastrophic, so the back buffer is the precondition that makes
WC adoptable rather than a complement to it.

**Decision.** The back buffer lands now, in slice 4, as `007` scheduled it.
Write-combining does **not** land with it, and the reason is a boundary the
research pass did not have: WC needs the kernel to map the framebuffer *itself*
with PAT bits, and D-45 keeps device holes out of the direct map precisely so
that devices are mapped deliberately. That mapping is slice 5's work. WC moves
to slice 5 with the argument intact — it is still gated on the back buffer, and
the back buffer now exists.

`Framebuffer` gains a shadow: one `u32` per pixel at the device's own stride.
Every write goes to both; every read comes from the shadow. `attach_back_buffer`
is separate from construction because the framebuffer is built during the boot
handoff, long before there is a heap — and it seeds the shadow by reading the
screen once, so attaching it mid-boot does not lose the lines already printed.
That single read is the last one the design permits.

**Rationale.** Splitting the two halves is not a retreat from §3.4's argument;
it is the argument applied in the right order. The dependency runs
back-buffer → WC, so doing the back buffer alone leaves nothing incorrect,
while doing WC alone would have made every scroll dramatically worse. The
research pass identified the coupling correctly and the sequencing loosely.

`try_reserve_exact` rather than `vec!`: a framebuffer shadow is megabytes, it
is the largest single allocation the kernel makes, and a console that cannot
get one should keep working rather than abort. Failure is reported and the old
read-back path stays.

**Consequences.** One on-machine check, and `001-boot.md`'s limitation is
retired. The helpers behind the check are `#[cfg(feature = "selftest")]` — the
three-pass gate D-22 gained in the audit caught them as dead code in the
configuration that ships, which is exactly what that pass exists for.

Writes now cost twice: once to the device, once to RAM. That is the trade, and
it is the right one — a scroll moves a screen's worth of pixels and previously
*read* every one of them uncached, while a glyph write touches a few hundred
words. WC in slice 5 will make the device half of every write cheaper still.
