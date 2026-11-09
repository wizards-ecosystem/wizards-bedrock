# D-382 Destructive cleanup retains directory authority through traversal

Status: Accepted, 2026-10-10. Amends D-263; closes Codex Security occurrence `occ_b5eed8ca35b5d6b6379fb621`.

**Context.** D-263 made `cargo xtask clean --all` accept only the lexical
repository `vm/` child, reject links and reparse points in the checked path,
canonicalize it twice and then call path-based `remove_dir_all`. Those checks
closed static redirection but retained a time-of-check/time-of-use boundary: a
same-authority actor could rename the validated repository ancestor, put an
outside tree at the same pathname and make the later destructive open select
that tree. Repeating canonicalization narrowed the window but did not retain
the object that had been authorized.

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

**Proof required.** A deterministic host witness must replace the repository
ancestor at the former validation/deletion boundary and keep an outside
sentinel byte-identical. Static final, nested and intermediate redirects must
be refused or unlinked without touching their targets. Nested ordinary state
must be removed while the `vm` root remains. Exhausted entry and depth budgets
must refuse before deleting their sentinel. Persistent mutations must restore
final-component following, ancestor-component following, ambient path removal
after dropping both capabilities, and unbounded entry and depth traversal.

**Evidence.** Against the old implementation, the synchronized ancestor-swap
witness deleted the replacement tree and failed reading its sentinel. The
repaired five-test cleanup group passes exact-root/nested removal, root and
nested redirects, an intermediate redirected ancestor, the same synchronized
swap, and both resource ceilings. Mutations 795, 796 and 1,773 through 1,775
each compile and fail their exact named witness; mutation 794 retains the
independent exact-root refusal. The restored tree passes check, Clippy,
formatting and documentation; the serial host suite passes 749 tests with 10
intentional ignores; all 2,030 public API items have compiler-resolved callers;
and supply-chain plus vendored-input verification account for 80 registry
crates, three artifacts and four runtime inputs. The change is confined to
host tooling, its dependency records and documentation, leaving kernel and
boot-image source unchanged.

**Alternatives.** Another metadata or canonical-path check leaves the same
gap before the destructive open. Calling the standard path remover is safe
against descendant symlink traversal on supported hosts but cannot bind its
initial root to an earlier application decision. A bespoke `openat`/NT native
layer would duplicate subtle platform code and require lifting the host
tooling's unsafe-code prohibition. Removing the `vm` root after emptying it
would add a second name-to-object race with no useful cleanup benefit.
