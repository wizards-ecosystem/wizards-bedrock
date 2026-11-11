# D-382 Destructive cleanup retains directory authority through traversal

Status: Accepted, 2026-10-10. Amends D-263; closes Codex Security occurrence `occ_b5eed8ca35b5d6b6379fb621`.

**Decision.** Cleanup first requires the exact lexical `root.join("vm")` target.
It opens the absolute repository path from its filesystem root one component
at a time through `fs_at`, disables link following on every relative open and
requires each opened handle to describe a real non-reparse directory. It then
opens only `vm` relative to the retained repository handle with the same
no-follow rule. Recursive traversal reads, opens, unlinks and removes every
descendant relative to already-open directory handles; an opened reparse point
is unlinked as a non-directory and is never traversed. The top-level `vm`
directory remains in place and empty, so no final ambient-path removal is
needed. An object renamed after it is opened remains the authorized object;
pathname replacement cannot redirect the operation to another object.

The traversal refuses after 100,000 entries or 256 directory levels rather
than permitting attacker-controlled work or stack depth. A concurrent writer
can therefore cause a bounded partial cleanup and an error, never broaden the
deletion authority. The safe `fs_at` adapter is a development-only dependency;
its locked closure is recorded in the inventory and deterministic SPDX while
project source retains `#![forbid(unsafe_code)]`.

A deterministic host witness replaces the repository ancestor at the former
validation/deletion boundary and keeps an outside sentinel byte-identical.
Static final, nested and intermediate redirects are refused or unlinked without
touching their targets, nested ordinary state is removed while the `vm` root
remains, and exhausted entry and depth budgets refuse before deleting their
sentinel. Mutations 795, 796 and 1,773 through 1,775 restore final-component
following, ancestor-component following, ambient path removal after dropping
both capabilities, and unbounded entry and depth traversal, each failing its
exact named witness; mutation 794 retains the independent exact-root refusal.

**Rationale.** D-263's lexical, link and double-canonicalization checks before
a path-based `remove_dir_all` left a time-of-check/time-of-use gap: a
same-authority actor could rename the validated ancestor and put an outside
tree at the same pathname. Another path check leaves that gap; the standard
path remover cannot bind its initial root to an earlier decision; a bespoke
`openat`/NT native layer would duplicate subtle platform code and require
lifting the host tooling's unsafe-code prohibition; and removing the emptied
`vm` root would add a second name-to-object race.
