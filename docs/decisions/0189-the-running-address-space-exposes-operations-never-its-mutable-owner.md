# D-189 The running address space exposes operations, never its mutable owner

Status: Accepted, 2026-08-27. Amends D-86 and closes PW-102.

**Decision.** The closure and the `SPACE` owner are private to `mem`. Production
exports the two operations it actually needs: map one guarded stack and map one
validated device extent. Neither returns or accepts an `AddressSpace`.

Machine-only mapping probes remain, but their cleanup authority is linear.
`map_owned_for_test` returns an `ActiveMapping` whose base and length are
private; `unmap_for_test` consumes that value and can therefore remove only the
fresh mapping which minted it. The stale-stack proof receives the same token
beside its `MappedStack`. User-copy setup can add a user mapping but has no
removal door. The six invalid-unmap probes move behind one fixed, argument-free
operation in `mem`, so a caller cannot substitute an owned kernel address and
cause the probe itself to remove it.

**Rationale.** D-86's `mem::with_active_space` handed any safe kernel module
`&mut AddressSpace` with `Reach::Everywhere`, including `reclaim(None)`, which
can free the tables the CPU is walking, and `unmap`, which can remove a live
stack; soundness depended on the convention that no caller did. `AddressSpace`
is the right owner while tables are under construction and for each isolated
user space; splitting its public methods would not make the live kernel
instance narrower. The authority boundary is the holder of that instance.
Operation-shaped entry points preserve D-85's token-bearing lock acquisition
while making reclaim and arbitrary removal unnameable to callers. A consumed
mapping token keeps the one legitimate machine cleanup without turning a test
API into the original hole under a new name.

**Alternatives.** Adding a runtime `is_active` refusal to
`AddressSpace::reclaim` would protect only one door and complicate reclamation
of user spaces without removing arbitrary unmap. A public guard with a reduced
trait could still be forgotten or widened and was more machinery than two
production operations need. Removing live mapping entirely would regress
guarded dynamic stacks and discovered device windows.

**Consequences.** Every former caller goes through a narrow operation, and the
existing mapping, user-copy, rollback, unmap, stale-stack and NVMe
device-window checks stay green. The host source proof requires the only
closure-shaped borrow to remain private, with a persistent mutation that exports
that function. No machine check or unsafe operation was added.

**Amended by D-353.** The name-specific source proof is replaced by a
signature inventory over `mem`: no item visible outside the module names the
running `AddressSpace`. Mutation 301 keeps its identity and now names that
inventory instead of the removed name search (PW-563).
