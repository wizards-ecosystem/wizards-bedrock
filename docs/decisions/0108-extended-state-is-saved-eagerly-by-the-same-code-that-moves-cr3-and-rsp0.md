# D-108 Extended state is saved eagerly, by the same code that moves `CR3` and `RSP0`

Status: Accepted (2026-08-24)

**Context.** D-82 decided the shape — eager, compacted, boot-sized, out of
line, arithmetic in `libs/` — and D-29 recorded the debt. `libs/x86-cpuid`'s
`xstate` module has been host-tested since slice 5, including the APX case that
made Linux delete its monotonicity assertions. Nothing saved anything. D-99
turned that from a gap into a bug: `RSP0` follows the thread, so two threads
can be in ring 3, so two threads can have floating-point state to lose.

**Decision.** `wost::arch::x86_64::xstate`, and the save sits in
`task::prepare` beside the `RSP0` write and the `CR3` install. Three things
that move with a thread, one place that moves them, for the reason D-99 gave:
it is the only place every switch passes through.

The save happens *before* the switch, not inside it. What makes that sound is
`+soft-float`: between the `xsavec` and the `switch` the kernel executes only
integer code, so there is nothing left in a vector register to lose. That build
flag was already load-bearing for trap entry; this is the second thing resting
on it, and the CI check that it stays is worth more than it was.

`XCR0` is x87, SSE and AVX (D-82). The buffer is `.bss`, one `CEILING`-byte
area per slot, 64-byte aligned — in `.bss` rather than on the heap because the
switch must not allocate and must not fail: a thread cannot be told "no room to
save your registers". The *size* is `CPUID`'s and is checked against the
ceiling at boot, so the array is room and not layout, which is what D-82 meant
by out of line.

**`XSAVEC` where the processor has it, `XSAVE` otherwise, `XRSTOR` for both.**
`XRSTOR` reads the header's `XCOMP_BV` to tell the forms apart, so one restore
serves either and there is no second code path to keep right. D-82 preferred
`XSAVES`; that needs `IA32_XSS` and supervisor state this kernel has none of,
and the component offsets `libs/x86-cpuid` computes are what CET will want, not
what saving needs. The size taken is the larger of the compacted layout's and
`CPUID.0DH.0:EBX`, so one ceiling covers whichever form turns out to be there.
This machine reports 832 bytes and `XCR0` 0x7.

**What the first attempt got wrong, because the check caught it.** Setting
`CR4.OSXSAVE` and writing `XCR0` is not enough to *execute* an SSE instruction:
`CR4.OSFXSR` gates that, and without it every one is a `#UD`. The kernel had
never executed one — `+soft-float` again — so nothing had ever asked. The probe
the check needed was the first, and it faulted on its first boot.
`enable_extended_state` now sets `OSFXSR` and `OSXMMEXCPT` and clears `CR0.EM`
as well.

**Consequences.** `xstate::write_vector_probe` / `read_vector_probe` are the
only things in this kernel that touch a vector register, and they exist so
`a_vector_register_survives_a_switch` can ask. Two threads write different
values to `xmm0` and each still has its own afterwards, which is only true if
the switch away saved and the switch back restored. Broken twice — the save
removed, and `XCR0` narrowed by a bit — and red both times. 112 checks.

Left owed: `#NM` (vector 7) is still reserved for a future XFD-based allocator
and nothing is built on it (D-82); the ceiling is a constant rather than a
growable allocation, which is the shape a wider `XCR0` would force a decision
about; and CET's shadow-stack pointers, which are the reason the component
offsets in `libs/x86-cpuid` are computed at all, have no user yet.
