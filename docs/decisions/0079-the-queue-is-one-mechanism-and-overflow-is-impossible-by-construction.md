# D-79 The queue is one mechanism, and overflow is impossible by construction

Status: Accepted, 2026-08-24.

**Context.** This is the second doc `002` owed before Phase 5. D-06 chose
submission/completion rings. `010` §2 refuted their recorded rationale, kept the
decision, and answered the four problems that decide whether a ring ABI is safe.

**Decision.** One ring mechanism, kernel-allocated pages, single owner.

- **Every operation posts exactly one terminal completion, always**,
  cancellation included. Cancellation is advisory.
- **The kernel owns the buffer.**
- **Completion-queue space is reserved at submission.** The CQ is a power of
  two and at least twice the SQ size.
- **Each entry is copied out exactly once and then validated.** The API enforces
  it: the validated copy is all the kernel can reach.
- **Ports and completion queues are the same object.** Zircon's four semantic
  wins move into the CQ: one-shot arming by default, explicit edge-versus-level,
  cancellation that purges queued entries, and exactly one waiter released per
  entry.
- Ring structures carry **explicit version fields**; the opcode surface stays
  minimal; there are **no privileged opcodes**.
- **No global FIFO across submitters.**
- The measured ordering replaces D-06's rationale: structural asynchrony ≈ 10×,
  total trap elimination ≈ 1.3× at the cost of a dedicated core.

**Rationale.** Each rule has a named failure behind it. `-EALREADY` in
io_uring's manual means "the caller must wait for the canceled request to post
its completion event". If cancellation could suppress a completion, one
operation would have two exit paths. Blocking in `Drop` is unsound because
leaking is safe, so a userspace-owned buffer has no sound lifetime; Windows RIO
states the rule kernel-side. io_uring's `CQ_NODROP` took an unbounded
kernel-memory liability for a misbehaving application. RIO's reservation-based
admission control makes overflow *structurally impossible*, a proof rather than
a mitigation. The double fetch has a CVE: CVE-2025-39816 is "always use
`READ_ONCE()` to read ring pointers."

Separate ports *and* completion queues give userspace two event planes to join.
That is why io_uring grew `poll_add`, `futex`, `waitid` and `msg_ring`. A Zircon
port already *is* a completion queue whose entries are events.

The austerity is io_uring's security record read correctly. The ~60% kernelCTF
figure is confirmed and Google still disables io_uring in 2026, but the
statistic is frozen *because* they disabled it. NVD returns 143 io_uring CVEs
and rising: 38 in 2024, 40 in 2025, 36 by partial 2026. They cluster in **newly
added** surface: `zcrx`, provided buffers, `ublk`, passthrough. By class: 50
race/TOCTOU, 23 use-after-free, 10 refcount. The risk is feature velocity on a
ring surface, not io_uring the name.

No global FIFO is the Scalable Commutativity Rule again. sv6's mail-server
benchmark shows an ordered socket forces one shared queue, and an unordered one
allows per-core queues.

**Alternatives.** Rejected: io_uring's overflow list (unbounded kernel memory);
userspace-registered buffers (no sound lifetime, since leaking is safe).
Rejected: separate ports and completion queues (the two-plane defect io_uring
spent four features patching). Rejected: validating in place (the double fetch,
with a 2025 CVE); a rich opcode surface (the CVE clustering).

**Consequences.**

- Slice 9 writes a TLA+ model before the implementation, per D-84.
- `libs/ring-proto` owns the index arithmetic and the protocol state machine.
- A wait with a timeout and a wait on two queues, open since Phase 4, fall out
  of this mechanism.
- **If the native ABI is a ring, "observe the syscalls" is not a valid basis for
  auditing, sandboxing or introspection.** ARMO's 2025 "Curing" rootkit ran
  command and control without a system call, and Falco, Tetragon and Microsoft
  Defender missed it. Every observability mechanism this kernel wants must be a
  ring feature, designed in from the start.
- `002`'s open-questions list loses its second entry.
