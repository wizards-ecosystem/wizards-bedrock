# D-108 Extended state is saved eagerly, by the same code that moves `CR3` and `RSP0`

Status: Accepted, 2026-08-24.

**Decision.** `wost::arch::x86_64::xstate` implements D-82's shape, with the
save in `task::prepare` beside the `RSP0` write and the `CR3` install. One place
moves all three things that follow a thread, since every switch passes through
it (D-99).

The save happens *before* the switch, not inside it. `+soft-float` makes that
sound: between the `xsavec` and the `switch` the kernel executes only integer
code, so no vector register holds anything to lose. Its CI check guards trap
entry and this.

`XCR0` is x87, SSE and AVX (D-82). The buffer is `.bss`, one `CEILING`-byte area
per slot, 64-byte aligned, because the switch must not allocate or fail: a
thread cannot be told "no room to save your registers". The *size* is `CPUID`'s,
checked against the ceiling at boot, so the array is room and not layout, as
D-82 meant by out of line.

**`XSAVEC` where the processor has it, `XSAVE` otherwise, `XRSTOR` for both.**
`XRSTOR` reads the header's `XCOMP_BV` to tell the forms apart, so one restore
serves either with no second code path. D-82 preferred `XSAVES`, which needs
`IA32_XSS` and supervisor state this kernel lacks. The size taken is the larger
of the compacted layout's and `CPUID.0DH.0:EBX`, so one ceiling covers either
form.

`enable_extended_state` sets `CR4.OSXSAVE`, `OSFXSR` and `OSXMMEXCPT` and clears
`CR0.EM`, because without `OSFXSR` every SSE instruction is a `#UD`.

**Rationale.** D-99 turned the missing save into a bug: two threads can be in
ring 3 and have floating-point state to lose. Saving where `CR3` and `RSP0`
move puts every per-thread hardware fact on the one path a switch cannot skip.

**Consequences.** `xstate::write_vector_probe` / `read_vector_probe` are the
only things in this kernel that touch a vector register. They exist so
`a_vector_register_survives_a_switch` can ask: two threads write different
values to `xmm0`, and each keeps its own only if the switch saved and restored.

Left owed: `#NM` (vector 7) stays reserved for a future XFD-based allocator,
with nothing built on it (D-82). The ceiling is a constant, not a growable
allocation, a shape a wider `XCR0` would force a decision about. And CET's
shadow-stack pointers, the reason `libs/x86-cpuid` computes component offsets at
all, have no user yet.
