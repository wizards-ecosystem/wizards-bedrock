# D-407 Native user execution without RDCL_NO uses paired roots and an exact entry enclave

Status: Accepted, 2026-10-14. Implemented with external qualification OPEN; amends D-45, D-81, D-88, D-106 and D-250; advances PW-580 and PW-706.

**Decision.** A native processor without guest-read affirmative `RDCL_NO` may
enter ring 3 only through a fully constructed KPTI pair. The pair has one
**full root**, containing the process's user mappings and the frozen kernel
higher half, and one **stripped root**, containing the same user mappings plus
only the entry enclave below. Lower-half user table subtrees are shared by the
pair, so a user mapping has one set of leaves and one owner; the two top-level
roots and the stripped enclave hierarchy are distinct owned page-table frames.
No kernel direct-map, heap, ordinary text or rodata, ordinary task stack,
allocator metadata, page-table alias, framebuffer or device mapping appears in
the stripped root.

The exact supervisor-only entry enclave is the page-rounded union of these
dedicated, linker-bounded objects and nothing else:

| Object | Stripped-root permission and purpose |
|---|---|
| interrupt-vector stubs plus the common entry/exit assembly | read-only and executable; the only instructions permitted before the full-root load or after the stripped-root load |
| the IDT and the GDT/TSS bytes hardware may read during a privilege crossing | read-only and NX in the stripped root; writable aliases, where required for `RSP0`, exist only in the full root |
| one CPU-local routing page | read-only and NX; contains only the current pair's full-root address, the full-root task-stack destination and public entry geometry, never a pointer to secret data or secret bytes |
| the CPU-local transition stack and every IST stack reachable before the full-root load | supervisor-only, writable and NX, each with its layout-owned lower guard absent from both roots; bytes are scrubbed before user execution |

Each object receives its own page-aligned linker range; a page containing any
unlisted symbol or padding outside the declared entry sections is not an
enclave page. The full root maps those same physical objects with the
permissions their kernel-side writers require. The enclave is pinned by one
boot-lifetime owner; an address-space pair owns only its references and its
page-table frames. A missing, extra, aliased, user-accessible, global,
writable-executable or differently backed enclave leaf refuses construction
before a binding can exist.

On entry from CPL 3, hardware lands on the guarded transition stack with
interrupts disabled. Assembly saves the architectural user frame, loads the
pair's full root into `CR3`, transfers the saved frame to the already-pinned
ordinary task stack and clears the complete transition-stack payload before
calling any Rust. No Rust body, handler pointer, allocator, lock or ordinary
kernel datum is reachable before that load. Entry from CPL 0 stays on the full
root and the current task stack.

On return to CPL 3, Rust finishes while the full root is active. The assembly
tail validates and copies only the user-visible register state and architected
`iretq` frame onto a freshly scrubbed transition stack, obtains the selected
stripped-root value, and performs its last access to the ordinary kernel stack
or other potentially secret memory before loading that root. After the `CR3`
write it executes only enclave instructions, reads only the sanitized entry
frame, performs the already-required `VERW` ordering, and executes `iretq`.
Any fault, nested entry or inconsistent pair before the user return fails on
the full root; there is no recovery path that resumes user mode on the full
root by accident.

PCID stays disabled. Enclave leaves and all caller-selected user leaves remain
non-global. Every full/stripped transition therefore uses an ordinary `CR3`
load with the existing architectural flush semantics; this decision makes no
performance claim and creates no deferred per-PCID invalidation state. D-250's
affirmative `RDCL_NO` and exact QEMU TCG reasons continue to authorize the
existing shared-root mode. They do not authorize KPTI by implication, and
KPTI readiness does not synthesize either reason. Admission selects one typed
mode, `shared with positive evidence` or `paired and isolated`, and refuses if
the selected mode's complete proof is absent.

The pair is one linear ownership unit. Construction preflights the enclave and
all user extents, allocates into a rollback ledger, publishes neither root
until both agree, and returns every frame exactly once on any refusal. A task
binding consumes the pair rather than a bare root. Scheduling and trap entry
install only the full root while kernel code can run; the stripped root is
loaded only by the final assembly exit. Retirement must prove that neither
root is installed or bindable before reclaiming either. If unmap, release or
root revalidation refuses, both roots and every still-owned frame remain
quarantined together. This extends D-88's translation debt and D-106's binding
evidence to the pair without weakening either.

**Relation to earlier decisions.** D-45's boot activation and kernel-owned
full root remain the authority from which the full half of every pair is
derived. D-81's decision against KPTI is reversed for the now-observed
no-`RDCL_NO` native target; its FRED-shaped entry and `VERW` requirements remain.
D-88's synchronous no-PCID flush before frame return now covers both roots and
their shared lower-half tables. D-106's installed address space becomes an
inseparable pair and its switch must leave kernel execution on the full member.
D-250 remains the only authority for shared higher-half execution and gains
the independent paired-isolation admission reason; no bit, signature or
hypervisor log is promoted into CPU evidence.

Host layout tests must accept only the exact dedicated
enclave ranges and permissions, then reject one missing page, one extra
higher-half leaf, one user/global/W+X bit, one non-identical backing frame, one
unguarded stack and one pair whose lower-half leaf differs. Allocation and
reclamation fault injection must stop after every acquired root, intermediate
table and enclave reference and prove the complete frame count returns or the
complete pair remains owned. The compiled-source contract must prove that the
entry `CR3` load precedes every Rust call and ordinary-stack access, that the
exit `CR3` load follows the last secret-capable access and complete stack
scrub, and that no other instruction door can install the stripped root.
The exact host/source witnesses are
`a_kpti_user_root_contains_only_the_entry_enclave`,
`a_kpti_pair_shares_one_identical_user_subtree`,
`a_kpti_pair_refusal_returns_every_frame`,
`a_kpti_pair_cleanup_retains_complete_authority`,
`kpti_entry_loads_full_root_before_rust` and
`kpti_exit_loads_stripped_root_after_scrub_and_last_secret_access`.

Machine checks, run with paired isolation forced even on QEMU, must report
`the user root contains only the entry enclave`, `every user entry reaches Rust
on the full root`, `every user return installs the stripped root`, `the entry
stack is guarded and scrubbed`, `a context switch moves one complete root
pair`, and `paired-space refusal and retirement conserve every frame`. The
root-order witnesses must cover the software gate, timer entry, a user page
fault, retirement from trap context and a switch between two distinct spaces.
Separate user probes must fault on representative kernel text, data, direct-map,
ordinary-stack and device addresses without preventing a fresh process from
continuing the suite. The no-PCID/non-global premise is read back rather than
assumed.

**Consequences.** The pair, enclave, guarded stacks, paired binding and reclaim,
assembly-only CR3 crossings and forced self-test selection are implemented.
Distinct boundary symbols may resolve to the same address in an optimized
build, so the address observation crosses a compiler black box before equality
checks, and the post-CR3 `VERW` selector lives in dedicated read-only enclave
data, leaving the scrubbed transition stack untouched. Mutations 2,048 through
2,061 retain the paired lower-half admission, exact enclave, shared subtree,
pair publication/cleanup, full-root Rust entry, stripped-root return, stack
scrub, atomic pair switch, frame conservation, optimized linker-address
handling, self-test image identity and post-root no-stack-write invariants,
each observed red against its named witness.

**2026-10-16 validation amendment.** Permanent mutation 196 removes the
shared-root return's `verw word ptr [rsp]`, which the forced-paired self-test
cannot observe. The host assembly contract now enumerates both required buffer
clears and binds each tally to its exact preceding operand: `verw word ptr
[rbx - 8]` for the paired exit and `verw word ptr [rsp]` for the shared exit.
Mutation 196 names that host contract directly. The stronger verifier's green
focused test and the earlier 290/290 boots are not a substitute for observing
the mutation red in the replacement complete sweep.

PW-580 and PW-706 remain open only for the complete current release sweep and
fresh self-test plus shipped VMware qualifications. Those records must show
that guest-read `0x44` selects the paired path, reaches ring 3 on the stripped
root, crosses through the exact enclave and returns without a leaked mapping
or retained secret. The unresolved reason Workstation logged `0x2f` may remain
a diagnostic question; it is neither required nor permitted to stand in for
that proof.

**Rationale.** D-81 declined KPTI expecting `RDCL_NO`, and D-250 then allowed
shared higher-half mappings to authorize ring 3 only on guest-read affirmative
`RDCL_NO` or the exact host-pinned QEMU TCG tuple. The VMware guest reads
architectural capabilities `0x44`, without `RDCL_NO`, although a Workstation log
described `0x2f`, and the cause is not established. That observation fires
PW-706: VMware is a supported native qualification target on which user
programs must run, and its guest has not claimed immunity from Meltdown-style
supervisor-data disclosure. The existing user root copies the complete kernel
higher half, so relaxing D-250 would put kernel text, data, the direct map,
stacks, page tables and device mappings back in speculative reach, and the
repair had to preserve the trap, scheduling, cleanup and translation-ownership
arguments rather than add an unmeasured alternate entry path.
The Workstation log, CPU model, host capability or a vendor-signed claim is not the
architectural value the guest executes under, and inventing `RDCL_NO` erases
the boundary. Clearing `GLOBAL`, relying on SMEP/SMAP or adding `LFENCE` leaves
supervisor data in the speculative address space; a trampoline on an ordinary
kernel page makes unrelated bytes part of the exception; mapping each user leaf
twice lets the views drift; PCID adds identifier lifetime and invalidation
obligations before the unoptimized path is proved; and switching roots in Rust
would reach Rust and its stack under the stripped root on entry or leave
secrets live after the switch on exit.
