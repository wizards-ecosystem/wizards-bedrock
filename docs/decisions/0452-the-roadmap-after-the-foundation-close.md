# D-452 The roadmap after the foundation close

Status: Accepted, 2026-11-09. Amends D-02, D-04, D-05, D-10, D-20, D-44,
D-70, D-73, D-114, D-154, D-183 and D-184; records `docs/021-roadmap.md`.

**Context.** Bedrock's phase list in `000-vision.md` stopped at a compiler
gate: from Phase 7 on, a kernel mechanism entered the register only with a
named Wizzy caller, and the register's `TRIGGER` rows were stopped work
(D-183, D-184). Wizzy's handoff has not arrived, and the kernel that boots
today runs one fixed program with no spawn, no channels between processes,
no write path and one processor. On 2026-11-07 the owner set eleven roadmap
rulings for this decision. On 2026-11-08 and 2026-11-09 the owner reviewed them
against research into capability kernels, production server kernels, driver
isolation, kernel memory accounting, tenancy, kernel extensibility and the
market for a new kernel, and changed several. This decision records the
result; `docs/021-roadmap.md` holds the order and the mechanism table.

**Decision.**

1. Bedrock is a general-purpose, secure kernel; Realm and Wizzy are its
   first users and the test of every feature (ruling 12, kept). Its position
   is the small, provably isolating base under servers: tenants are cheap
   Bedrock processes or Linux guests in boxes Bedrock controls, and Linux is
   hosted, never imitated. The destination in D-05 (a full OS with a
   browser and Python on this kernel) is reached through hosted guests and
   Realm, not through kernel scope.
2. No kernel mechanism waits for a Wizzy or Realm caller (rulings 13 and 19,
   superseded). Each phase ships with Rust witness programs that exercise
   its mechanisms and are retired when Realm's Wizzy programs replace them.
   D-183's rule that first-party product software is Wizzy stands; a witness
   is a check, not a userland. D-184's register remains the record of the
   foundation close and no longer admits or refuses new mechanisms.
3. Application processors start right after Phase 7's process and IPC
   basics, with fine-grained locking from the first bring-up (ruling 14,
   amended; D-154's Phase 12 placement and D-70's big-lock first step end).
   Server and desktop work then proceed in parallel (ruling 15, kept).
4. No real-hardware work before the laptop phase (ruling 16, superseded).
   QEMU is the loop and VMware the milestone (D-18) until then.
5. A cheap process per tenant is the product; sub-process domains are a
   research track, designed on paper, with a written threat model, and never
   called tenant isolation unless they equal the process boundary (rulings
   17 and 18, kept).
6. Code loading is write, seal, execute: seal consumes the handle and returns
   a new one; there is no unseal and no dual mapping; code generation is a
   transferable quota object; the kernel chooses and randomizes executable
   placement; seal refuses while any live write reference exists (ruling 20,
   kept; replaces D-114 item 4).
7. The network stack is a program on top of the kernel, never inside it
   (ruling 21, amended; D-20's in-kernel smoltcp ends). The kernel hands out
   device queues; a stack service holds a platform-independent protocol
   core; programs see one socket interface; the queues between driver,
   stack and program are batched shared memory. Ruling 21's driver order is
   kept: a CSPRNG first, a polled e1000e, MSI-X through interrupt
   remapping, vmxnet3 and cloud NICs behind named deployments, USB Ethernet
   for the laptop.
8. x86_64 only (ruling 22, kept; amends D-02's "ports kept possible" to
   "seams kept").
9. The kernel parses no executable format: a user-space loader maps a sealed
   memory object and starts a thread.
10. The ring (D-06) gains a synchronous call and reply path; processes live
    in kernel-enforced nesting groups that carry policy, limits and kill.
11. Each group owns a kernel-memory arena of frames its parent donated. All
    kernel objects the group creates are carved there by the kernel's
    allocator; the kernel never allocates from a shared pool for a user
    request; a dead group's arena is freed whole. D-44's "not an accounting
    mechanism" now reads: the type discipline is unchanged, and accounting
    lives at the group arena.
12. Drivers are safe Rust over `wost` tokens, sharded per core, run as fenced
    processes for storage, network, graphics and radio and inside a budgeted
    kernel executor for small latency-sensitive devices; rings and buffers
    are client-owned and the intent log is part of the ring contract. D-04's
    monolithic-modular shape becomes a framekernel with driver processes.
13. Wizzy is the kernel's extension language: a typed intermediate form
    checked in the kernel by a small type, effect and linearity checker, with
    eBPF bytecode accepted through a second front end. Tenants may install
    schedulers for their own groups under enforced budgets, and a group may
    run deterministically with kernel-served time and a completion log.
14. A content-addressed object store on raw NVMe precedes WizFS and holds
    tenant snapshots and images (amends D-73's order, not its destination).
15. A bounded user-space runner later translates a published list of Linux
    system calls for static server binaries, with fork, control groups,
    ptrace and io_uring never offered; full compatibility is ruled out.
    This is D-10's translation layer, bounded.
16. Public text does not name the external actor runtime or its author
    (ruling 23, kept).

**Consequences.** `000-vision.md`'s phases 7 to 13 are replaced by `021`'s
order. `016` keeps PW-006 and PW-010 live and its `TRIGGER` rows become
rows of `021`'s phases. Each phase closes with a multi-processor QEMU
self-test, a VMware boot, a mutation per new check (D-28) and a model where
it adds concurrency (D-112). Realm records its own position in its own log
and is not cited here. The name collision with another deterministic
hypervisor project is an open item before anything public.

**Alternatives.** Keeping the compiler gate on kernel work left the kernel
idle until Wizzy's handoff and shaped mechanisms around guesses about a
caller that does not exist yet. Putting SMP after the first Wizzy programs
kept the first userland on the simplest kernel at the price every studied
retrofit paid in years. Binary Linux compatibility in the kernel would make
Bedrock Linux-shaped and competes with a four-year head start elsewhere. A
shared kernel heap with quotas is simpler to start and took the largest
kernel eight years and two rewrites to account honestly.
