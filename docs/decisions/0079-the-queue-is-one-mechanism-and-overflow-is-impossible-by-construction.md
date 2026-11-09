# D-79 The queue is one mechanism, and overflow is impossible by construction

Status: Accepted, 2026-08-24.

**Context.** The second doc `002` owed before Phase 5. D-06 chose
submission/completion rings and `010` §2 refuted the rationale recorded for
them while confirming the decision, then supplied answers to the four problems
that decide whether a ring ABI is safe.

**Decision.** One ring mechanism, kernel-allocated pages, single owner.

- **Every operation posts exactly one terminal completion, always** —
  cancellation included. Cancellation is advisory.
- **The kernel owns the buffer.**
- **Completion-queue space is reserved at submission.** The CQ is a power of
  two and at least twice the SQ size.
- **Each entry is copied out exactly once and then validated**, enforced by the
  shape of the API rather than by discipline: the validated copy is the only
  thing the kernel can reach.
- **Ports and completion queues are the same object.** Zircon's four semantic
  wins move into the CQ: one-shot arming by default, edge-versus-level as an
  explicit flag, cancellation that purges already-queued entries, and exactly
  one waiter released per entry.
- Ring structures carry **explicit version fields**; the opcode surface stays
  minimal; there are **no privileged opcodes**.
- **No global FIFO across submitters.**
- The rationale recorded in D-06 is replaced by the measured ordering:
  structural asynchrony ≈ 10×, total trap elimination ≈ 1.3× and it costs a
  dedicated core.

**Rationale.** Each rule has a named failure behind it. `-EALREADY` in
io_uring's own manual means "the caller must wait for the canceled request to
post its completion event", so any design where cancellation might suppress a
completion has two exit paths for one operation. Blocking in `Drop` is unsound
because leaking is safe, so a userspace-owned buffer has no sound lifetime;
Windows RIO states the rule kernel-side. io_uring's `CQ_NODROP` took an
unbounded kernel-memory liability for a misbehaving application, while RIO's
reservation-based admission control makes overflow *structurally impossible* —
one of these is a mitigation and the other is a proof. And the double fetch is
not theoretical: CVE-2025-39816 is literally "always use `READ_ONCE()` to read
ring pointers."

Building ports *and* completion queues would give userspace two event planes to
join, which is precisely why io_uring grew `poll_add`, `futex`, `waitid` and
`msg_ring`. A Zircon port already *is* a completion queue whose entries are
events.

The austerity is io_uring's security record read correctly. The ~60% kernelCTF
figure is confirmed and Google still disables io_uring in 2026, but the
statistic is frozen *because* they disabled it. NVD returns 143 io_uring CVEs
and rising — 38 in 2024, 40 in 2025, 36 by partial 2026 — clustering in
**newly added** surface: `zcrx`, provided buffers, `ublk`, passthrough. By
class: 50 race/TOCTOU, 23 use-after-free, 10 refcount. The lesson is feature
velocity on a ring surface, not io_uring the name.

No global FIFO is the Scalable Commutativity Rule again: sv6's mail-server
benchmark shows an ordered socket forces one shared queue while an unordered
one allows per-core queues.

**Alternatives.** io_uring's overflow list (rejected: unbounded kernel memory
for a misbehaving process); userspace-registered buffers (rejected: no sound
lifetime in Rust, because leaking is safe); separate ports and completion
queues (rejected: two event planes is the defect io_uring spent four features
patching); validating in place instead of copying out (rejected: that is the
double fetch, and it has a 2025 CVE); a rich opcode surface (rejected on the
CVE clustering).

**Consequences.**

- Slice 9 writes a TLA+ model before the implementation, per D-84.
- `libs/ring-proto` owns the index arithmetic and the protocol state machine.
- A wait with a timeout and a wait on two queues — both left open by Phase 4 —
  fall out of this mechanism rather than being built separately.
- **If the native ABI is a ring, "observe the syscalls" is not a valid basis
  for auditing, sandboxing or introspection.** ARMO's 2025 "Curing" rootkit did
  command and control without making a single system call, and Falco, Tetragon
  and Microsoft Defender were all blind to it. Every observability mechanism
  this kernel will ever want must be a ring feature, designed in from the
  start.
- `002`'s open-questions list loses its second entry.
