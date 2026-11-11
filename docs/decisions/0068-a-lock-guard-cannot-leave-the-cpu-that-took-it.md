# D-68 A lock guard cannot leave the CPU that took it

Status: Accepted, 2026-08-23.

**Decision.** At adoption both guards carried a private
`PhantomData<*const ()>`, which made them `!Send` and `!Sync`. D-223 later
centralizes the spin guard's same fact in the `InterruptState` it owns; the
thread-lock guard keeps that field directly.

**Rationale.** A guard released on another CPU would restore a flag that
belongs to the CPU that cleared it, leaving that CPU interruptible inside
its critical section. One line each, no runtime cost, and a class closed
before it opens; it is the stable-Rust form of Asterinas's `impl !Send`,
built like `ThreadContext` (D-62). D-223 makes the compiler refusal a
durable host check over both production guard types.

**Consequences.** The type system pins guards to their acquiring context,
not the absence of a second CPU. The SMP debt in D-56 shrinks by one item.
