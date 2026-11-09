# D-208 Mutation sweeps run from a journalled tracked-file copy

Status: Accepted, 2026-08-28. Amends D-119, D-134 and D-175; closes PW-502.

**Context.** D-119 restores a mutated file before any ordinary error may leave
the runner, but a process killed between the write and restoration has no Rust
error path to execute. The live checkout and its index are therefore exposed
for the entire build and boot behind every mutation. A 157-entry phase-close
sweep turns that small interval into hours of repeated risk, and an interrupted
run starts again at mutation one because it records no durable progress.

**Decision.** A mutation sweep requires a clean `HEAD`, inventories Git's
tracked files with optional index refresh disabled, and records the commit plus
exact source and index digests. It copies only those tracked files beneath
`target/mutate/<commit>-<registry>/workspace`, gives that copy a private Cargo
target directory, and directs every host build, image build and QEMU boot there.
The source checkout is read-only input to the runner. It is verified against
the recorded digests when a sweep starts, resumes and finishes.

One OS advisory lock covers the mutation state root. The lock file may remain,
but the kernel releases its lock when the process dies; a live second runner is
refused while a killed runner can be resumed without deleting a sentinel or
guessing whether its PID was reused. The lock is not inherited as a protocol
by build or QEMU children.

Progress uses two alternating, checksummed journal slots. Each complete slot
names the source commit, mutation-registry digest, source and index digests,
generation, active mutation and every completed exact outcome. The newer valid
generation wins; if a kill tears its write, the prior valid slot remains. The
active mutation is journalled and synced before its sandbox file changes. On
resume that one file is first restored from the still-verified source and
checked byte-for-byte, then the same mutation runs again. An outcome is synced
only after restoration is verified. Completed mutations are not repeated.

At a completed sweep, a durable result summary replaces the resumable slots;
the copied workspace and private build cache may remain under ignored `target/`
for the next invocation. No recursive cleanup is required for correctness.

**Rationale.** A signal-safe restoration handler cannot cover termination,
power loss or a killed process, and promising that every exit runs a destructor
would restate the defect. Moving the only writes out of the checkout makes
interruption harmless by construction. A tracked-file copy is smaller and
less stateful than `git worktree`: it neither changes `.git/worktrees` nor
requires cleanup of Git metadata after a crash. Alternating whole-state slots
avoid depending on rename-over-existing semantics, which differ on Windows,
while retaining the last known complete state.

**Alternatives.** A `Drop` restoration guard was rejected because process death
does not run it. A signal handler was rejected because filesystem and allocator
work is not signal-safe and Windows termination is not the Unix signal model.
An isolated Git worktree was rejected because it protects source bytes but adds
repository metadata which an interrupted cleanup can strand. A fresh copy per
mutation was rejected because it discards Cargo's cache and durable progress
without buying more isolation than one commit-pinned copy already provides.
An unchecksummed single journal was rejected because the interruption being
handled can occur while that journal itself is being written.

**Consequences.** `mutation_workspace::Session` owns the advisory lock, exact
Git baseline, immutable tracked-file snapshot, writable workspace, private
build directory and alternating journal. The runner restores an interrupted
active file before rerunning it, records an outcome only after exact restore,
and verifies the source plus index before retiring the journal. Host witnesses
also receive the private Cargo target explicitly.

Real helper processes were killed after announcing both build and boot work.
A live second lock was refused; after each kill the OS-released lock was
reacquired, the active journal reloaded and the sandbox restored while fixture
source and index bytes remained exact. A real temporary Git repository proved
that only tracked files reach a distinct workspace and that ignored files and
`.git` do not. A two-slot test selected the newest complete generation and fell
back after its newest slot was torn. Redirecting the workspace to the checkout,
bypassing the private Cargo target, bypassing the lock, omitting restore and
selecting the oldest journal each made only its named test red. Five persistent
host mutations retain those witnesses, bringing the registry to 162.
