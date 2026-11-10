# D-108 Extended state is saved eagerly, by the same code that moves `CR3` and `RSP0`

Status: Accepted, 2026-08-24.

**Context.** D-82 decided the shape (eager, compacted, boot-sized, out of line,
arithmetic in `libs/`) and D-29 recorded the debt. `libs/x86-cpuid`'s `xstate`
module has been host-tested since slice 5, including the APX case that made
Linux delete its monotonicity assertions. Nothing saved anything. D-99 turned
that gap into a bug: `RSP0` follows the thread, so two threads can be in ring 3
and have floating-point state to lose.

**Decision.** `wost::arch::x86_64::xstate`, with the save in `task::prepare`
beside the `RSP0` write and the `CR3` install. One place moves all three things
that follow a thread, since every switch passes through it (D-99).

The save happens *before* the switch, not inside it. `+soft-float` makes that
sound: between the `xsavec` and the `switch` the kernel executes only integer
code, so no vector register holds anything to lose. Trap entry already rested on
that build flag, so its CI check now guards two things.

`XCR0` is x87, SSE and AVX (D-82). The buffer is `.bss`, one `CEILING`-byte area
per slot, 64-byte aligned. It sits in `.bss`, not the heap, because the switch
must not allocate or fail: a thread cannot be told "no room to save your
registers". The *size* is `CPUID`'s, checked against the ceiling at boot, so the
array is room and not layout, as D-82 meant by out of line.

**`XSAVEC` where the processor has it, `XSAVE` otherwise, `XRSTOR` for both.**
`XRSTOR` reads the header's `XCOMP_BV` to tell the forms apart, so one restore
serves either with no second code path. D-82 preferred `XSAVES`, which needs
`IA32_XSS` and supervisor state this kernel lacks. The component offsets
`libs/x86-cpuid` computes are what CET will want, not what saving needs. The
size taken is the larger of the compacted layout's and `CPUID.0DH.0:EBX`, so one
ceiling covers either form. This machine reports 832 bytes and `XCR0` 0x7.

**What the first attempt got wrong, because the check caught it.** Setting
`CR4.OSXSAVE` and writing `XCR0` is not enough to *execute* an SSE instruction.
`CR4.OSFXSR` gates that; without it every one is a `#UD`. The kernel had never
executed one (`+soft-float` again). The check's probe was the first, and it
faulted on its first boot. `enable_extended_state` now also sets `OSFXSR` and
`OSXMMEXCPT` and clears `CR0.EM`.

**Consequences.** `xstate::write_vector_probe` / `read_vector_probe` are the
only things in this kernel that touch a vector register. They exist so
`a_vector_register_survives_a_switch` can ask. Two threads write different
values to `xmm0`, and each still has its own afterwards only if the switch away
saved and the switch back restored. It was broken twice (the save removed, and
`XCR0` narrowed by a bit) and went red both times. 112 checks.

Left owed: `#NM` (vector 7) stays reserved for a future XFD-based allocator,
with nothing built on it (D-82). The ceiling is a constant, not a growable
allocation, a shape a wider `XCR0` would force a decision about. And CET's
shadow-stack pointers, the reason `libs/x86-cpuid` computes component offsets at
all, have no user yet.
