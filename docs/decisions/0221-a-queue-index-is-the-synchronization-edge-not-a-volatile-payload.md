# D-221 A queue index is the synchronization edge, not a volatile payload

Status: Accepted (2026-08-29), amends D-79, D-84, D-112, D-113 and D-115; closes PW-401

**Context.** D-79 required `READ_ONCE()`-shaped access, D-113 copied each
shared entry once, and the Phase 5 implementation put a spin lock around the
kernel's `Shared` handle. None of those facts ordered ring 3. `Shared::read_at`
and `write_at` forwarded to ordinary `copy_nonoverlapping`; the compiler could
merge, move or elide accesses to memory an external agent changes. The lock
serialized kernel callers only. The current one-CPU trap route and x86 store
ordering hid the gap, while an SMP kernel or compiled runtime would have had
no stated happens-before edge between an entry and the index publishing it.

D-84 had proposed a Shuttle test which weakened `Release`/`Acquire`. D-112
records the measured result: it passed because Shuttle explores schedules
under sequential consistency, not weak-memory visibility. Keeping that test as
the proof would repeat the exact false oracle already found in Phase 5.

**Decision.** Each queue direction is SPSC and has one fixed four-step
contract. The producer volatile-writes a complete entry and any associated
data, then release-stores its naturally aligned 32-bit tail. The consumer
acquire-loads that tail before volatile-copying the entry exactly once, and
release-stores its head only after it is finished with the copy. The producer
acquire-loads the head before reusing the slot. Counters remain little-endian
free-running `u32`s; their existing wrapping validation remains separate from
memory ordering.

The index access itself is atomic. Volatile prevents invention, merging and
elision of payload accesses but supplies no ordering; a fence around an
ordinary or volatile index is not a substitute for an atomic index which
synchronizes with its peer. `wost::mm::Shared` therefore exposes bounded,
aligned `load_acquire_u32` and `store_release_u32` operations with no caller-
selected ordering, plus volatile byte-copy operations. Its former ordinary
shared-copy methods are gone. `kernel::queue` can express only the four
protocol edges through those named methods. Pages and private protocol state
share one bounded lock, making concurrent kernel completion paths one logical
producer; a second CPU cannot advance and publish tail 2 before the first has
written slot 0. The time page also moves onto the volatile payload boundary,
while retaining its distinct sequence-counter fences.

The hand-emitted x86-64 client has no compiler and uses naturally aligned
`mov`s, which implement acquire loads and release stores under x86 TSO. It now
acquires `completion_tail` before reading the completion and releases
`completion_head` after validation, in addition to releasing its submission
tail. The future compiled Wizzy runtime must express the same operations with
its platform atomics and volatile accessors; application code never selects an
order or touches the layout.

**Proof.** A four-event host model enumerates every order of payload
visibility, index visibility, index read and payload read. Observing a new
index with an old payload requires `index-visible < index-read` and
`payload-read < payload-visible`. Release adds `payload-visible <
index-visible`; acquire adds `index-read < payload-read`; together the four
constraints form an impossible cycle. Weakening either edge admits a concrete
order. The check reads the actual `wost` method bodies and the queue helpers,
so it is not a detached model of a protocol the implementation need not use.
A separate source check forbids the ordinary-copy bypass and pins both volatile
operations. The generated-client check pins payload, SQ release, CQ acquire,
completion reads and CQ release in execution order.

**Alternatives.** Treating the trap or spin lock as the barrier was rejected
because userspace does not take it and another CPU need not enter the same
critical section. Volatile indices were rejected because volatile is an
access property, not inter-thread ordering. `SeqCst` everywhere would be
correct but would erase the ownership-specific protocol and impose stronger
portable instructions than the proof needs. Making every entry field atomic
would still allow mixed-version records and obscure the copy-once validation
boundary; publication protects the volatile record as a unit. A probabilistic
stress test or x86 disassembly alone cannot prove the weak-memory property, and
Shuttle has already demonstrated that it cannot model it.

**Consequences.** The shared-memory wrapper grows two reviewed unsafe lines
and remains inside its 380-line budget at 370. Five checks were each broken
against their exact defect: release and acquire were independently weakened to
`Relaxed`, the volatile store became ordinary, and the real client acquired
its own head instead of the completion tail; splitting completion state from
page publication recreated the two-producer hole. Each failed only its named
host witness; the restored transformations persist as mutations 227 through 231.
The full gate and ring-3 boot result are recorded in PW-401's closing row.
