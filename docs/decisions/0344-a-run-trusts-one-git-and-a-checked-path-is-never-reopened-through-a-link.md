# D-344 A run trusts one Git, and a checked path is never reopened through a link

Status: Accepted (2026-09-28), amends D-218 and D-330; closes PW-553, PW-554 and PW-555

**Context.** The canceled Daybreak Deep scan retained three tool-trust
findings that reproduce on the current source. Every source identity,
clean-checkout test and parent-diff relation came from whichever `git` was
first on `PATH` when each query spawned, and no record named the executable
that answered, so a directory prepended to `PATH` could certify a modified
checkout as clean without leaving a trace. The qualification reader checked
that an artifact path named one real file and then opened it with an ordinary
open, which follows a link; a producer that could write the scan or grade
directory could swap a link in between and have an operator-readable file
copied into the records. Other qualification, evidence, supply-chain, vendor,
API-audit, image and QEMU readers paired the same check with the same open.
Separately, the compiler-contract probes created predictable directories under
the system temporary directory with `create_dir_all`, so a directory or link
placed first at that name received the probe's manifest and source.

**Decision.** A run resolves Git once from `PATH`, canonicalizes it, measures
its bytes and requires `git --version` to name Git; every Git query spawns
that exact file with the isolated Git environment. Every qualification record
and the evidence manifest bind `tool.git.path`, `tool.git.version`,
`tool.git.bytes` and `tool.git.sha256` beside the source commit and tree, and
the executable is measured again immediately before a record or manifest is
committed. Evidence verification recomputes those fields on the verifying
host, so a bundle that another Git produced does not verify. The VMware
generation receipt keeps only its source identity, because qualification
establishes that source again with its own Git before sealing. Qualification
format v4 and evidence format v8 refuse records without the binding.

A checked path is read through one primitive that does not follow a
final-component link: on Windows it opens the entry itself with
`FILE_FLAG_OPEN_REPARSE_POINT`, and elsewhere the opened handle must match the
directory entry's device and inode. Either way the handle must describe one
real file before any byte is read. Every function that checks a path and then
reads it uses that primitive, and a syntax check refuses any production
function that pairs a path check with `File::open`. Contract probes live under
the checkout's `target/contract-probes`, and each probe directory is created
exclusively, skipping any name already present.

**Evidence.** On 2026-09-28 the Git tests passed: a query spawns the resolved
canonical path, a non-Git decoy first on the search path is refused, Cargo is
refused by its version identity, an executable changed after it answered
blocks publication and each recorded field is validated. A syntax contract
proved that every function binding a source commit also names the resolved
Git, that nothing outside the Git module spawns `git` by name, and that both
publishers measure Git before they commit. A link swapped in after the path
check and swapped back after the open was refused. The reopen guard then found
three further functions, `publish_image`, `publish_namespace` and
`capture_vmware_file_with`, which now use the primitive. A probe skipped an
occupying link and left its target untouched. Mutations 1,482 through 1,502
each weaken one of these rules and each failed its exact witness before
byte-for-byte restoration; mutations 102, 800, 805 and 1,223 were re-anchored
to the converted code and failed their witnesses again.

**Alternatives.** Pinning one Git build by digest, as D-342 pins
cargo-mutants, was rejected because Git updates independently of this project;
a run needs its Git named and stable, and evidence verification already
requires the verifying host's Git to match. Refusing a Git outside a fixed
installation directory was rejected because installation layouts vary. An
open relative to a retained parent handle would also close a race on the
parent directory itself, but the standard library offers none on Windows; the
remaining race needs a producer that can replace the whole directory between
the check and the open with one holding a file of the same fixed name. Moving
every host-test fixture out of the system temporary directory was not done:
on the qualified Windows host that directory is private to the account, so
only the contract probes, which run Cargo and remove their tree, moved. A
host with a shared temporary directory becomes relevant only under PW-708.
