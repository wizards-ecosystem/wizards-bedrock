# D-347 A bounded command ends with every process it started

Status: Accepted, 2026-09-28. Amends D-264; closes PW-013.

**Context.** D-264 bounded each external child's capture and exit but stopped
only the direct child, and left delegated process trees to PW-013's trigger.
That trigger has fired. The mutation sweep runs each host witness as `cargo
test`, and D-342's seal runs `cargo test -p xtask --doc`; both start compiler
and test processes. The canceled Daybreak scan retained the consequence: a
witness that timed out lost only its `cargo` process, and a descendant could
keep running into the next mutation.

**Decision.** On Windows every bounded command runs in its own job object.
`command-group` 5.0.1 creates the child suspended, assigns it to the job and
only then resumes it, so nothing the child starts can escape the job. The
helper holds the child in one value whose drop terminates the whole job, and
a timeout terminates the job as well, so the job ends whenever the command
completes, times out, overruns its capture limit or cannot be observed. On
other hosts the helper still stops only the direct child; process groups there
belong with PW-708's non-Windows gate.

The crate is pinned exactly and used only by `xtask` on Windows. Its Windows
backend is `winapi`, a raw FFI crate with no procedural macros, so it stays
inside the API audit's syntactic trust boundary, which D-275 closes to every
procedural-macro package in the active graph. With its `winapi` support crates
and the unix-only `nix`, it adds five locked packages, all MIT or dual MIT and
Apache-2.0, recorded as development inputs; `winapi` and its x86_64 import
library carry the two reviewed build scripts. Its std interface cannot request
kill-on-close, so a hard external kill of `xtask` itself still leaves the tree;
every exit `xtask` controls, a panic included, terminates it.

**Evidence.** On 2026-09-29 rustc 1.97.1 linked `winapi`'s import library with
no `dlltool` on `PATH`, and the API audit accepted the graph with no
procedural-macro package and the two new build scripts reviewed. Two fixtures
start a grandchild that holds a file exclusively for a minute. After a normal
completion and after an 8 s timeout, the grandchild released the file within
the tests' 15 s bound. The existing bounded-process tests passed unchanged.
Mutation 1,522 skips termination at drop and fails the completed-command
fixture; mutation 1,523 makes termination a no-op and fails the timed-out
fixture; each restored byte for byte. Mutation 889 moved to the reformatted
capture check and failed its witness again. Mutation 1,521, which removed a
`process-wrap` job wrapper this decision no longer uses, is retired. The
supply-chain check verified the updated inventory and regenerated SBOM.

**Alternatives.** `process-wrap` 10 and `win32job` 2 were rejected because both
build on the modern `windows` crate, whose COM bindings pull in the
`windows-implement` and `windows-interface` procedural macros; the API audit
refuses a procedural-macro package in the active graph because syntactic
reachability cannot see what a macro expands to (D-275). `win32job` also
assigns an already running child to its job, which the child can outrun by
spawning first. Calling the job APIs directly was rejected because `xtask`
forbids `unsafe` and `wost` is the only crate that may write it. `taskkill /T`
was rejected because it follows parent links and misses a descendant whose
parent has already exited.
