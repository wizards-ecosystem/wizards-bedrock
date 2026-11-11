# D-48 The console scrolls out of RAM

Status: Accepted, 2026-08-22.

**Decision.** The back buffer lands now, in slice 4. Write-combining does
**not** land with it: WC needs the kernel to map the framebuffer *itself*
with PAT bits, and D-45 keeps device holes out of the direct map, so WC
moves to slice 5's device mapping, still gated on the back buffer.

`Framebuffer` gains a shadow: one `u32` per pixel at the device's own
stride. Every write goes to both; every read comes from the shadow.
`attach_back_buffer` is separate from construction because the framebuffer
is built during the boot handoff, long before there is a heap. It seeds the
shadow by reading the screen once, so attaching it mid-boot keeps the lines
already printed. That single read is the last one the design permits.

The shadow is allocated with `try_reserve_exact`, not `vec!`: it is the
largest single allocation the kernel makes, and a console that cannot get
one should keep working, not abort. Failure is reported and the old
read-back path stays.

**Rationale.** WC makes writes fast and reads catastrophic, so the back
buffer is the precondition for WC (`008-research-notes-2.md` §3.4). The
dependency runs back-buffer → WC: the back buffer alone leaves nothing
incorrect, while WC alone would have made every scroll far worse.

**Consequences.** `001-boot.md`'s read-back limitation is retired. Writes
cost twice, once to the device and once to RAM, against a scroll that
previously *read* every pixel uncached.
