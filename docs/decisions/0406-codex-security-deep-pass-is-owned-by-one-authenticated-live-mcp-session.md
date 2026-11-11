# D-406 Codex Security Deep PASS is owned by one authenticated live MCP session

Status: Accepted, 2026-10-14. Amends D-330, D-344, D-347 and D-373; advances PW-009, PW-026 and PW-028.

**Decision.** `qualify-security-deep --run` is the sole Deep PASS authority.
`--begin` and `--verify=DIR` remain diagnostic and their `DeepAttempt`
capability has no PASS method. The owned path uses a distinct non-clone
capability, is initially Windows-only, and starts the exact pinned Codex
Security 0.1.32 MCP server with a hash-pinned Node runtime in a D-347 Job.
The Job owns the server and every Python and Codex descendant from suspended
creation through clean exit, timeout, output refusal or dialogue failure. The
same bounded stdio session initializes MCP, binds the exact pinned provider tool
inventory containing exactly one required start and one completion tool, starts
exactly one new repository-wide Deep scan, waits for its canonical
manifest, and completes exactly that scan once. Reply IDs, server identity,
protocol, tool names, errors, notifications, paths, target, revision, status and
warnings are checked strictly. Text returned for display never controls a path
or verdict; only validated structured content does.

The request format advances to v2, qualification to v6 and evidence to v10.
Environment policy `xtask-owned-codex-security-hermetic-v4` and execution
authority `xtask-mcp-wrapper-windows-job-v3` bind the Windows Job, MCP protocol
and tool sequence, exact model/reasoning policy, managed read-only worker
sandbox, plugin manifest and tree, Node, Python, the complete adjacent Codex
runtime, Git and the source revision. Every
manifest-listed plugin file is verified and copied into a private mirror; only
that mirror is executed, and Python bytecode is redirected into private temp,
so unlisted files beside the installed package are not loadable inputs. The
plugin, executables and source are measured before use and again after the
completed result. Provider/configuration, alternate-provider, proxy and custom
CA overrides are removed; `CODEX_CLI_PATH`, `PYTHON`,
`CODEX_SECURITY_GIT`, the pinned Deep defaults and the one selected
authenticated Codex account channel are explicit. Missing authentication or
identity drift fails before PASS instead of falling back to another provider.
Every Git child receives only the fixed Git/locale environment, runs from the
measured executable's directory, disables system configuration, reads an empty
global configuration, and binds absolute Git-directory and worktree paths, so
loader, shell, `PATH`, caller-profile and repository-discovery inputs cannot
redirect the query. Fixed helper surfaces such as hooks, fsmonitor, pagers,
diffs and credentials are disabled. Before each query a bounded effective
local/included/worktree configuration read enumerates every content-filter
driver and command-scope overrides disable its clean, smudge and process
programs; malformed or oversized configuration returns a guaranteed-invalid
command and fails closed. Linked worktrees and `.git` gitfiles remain valid.
As elsewhere in this decision, a same-account rewrite between that discovery
and the immediately following query is outside the release-evidence threat
model.
Codex itself receives a private `CODEX_HOME` containing no ambient rules,
skills, plugins, memories or session state. Its sole authentication input is an
NTFS hard-link projection of the existing ChatGPT `auth.json`, never a copied or
reserialized credential. A no-share-delete source handle remains open for the
projection lifetime while allowing in-place token refresh; volume, file identity
and link count are checked before use and again when the private name is removed.
The exact-pinned development-only `winapi-util` safe wrapper obtains those
Windows handle facts without admitting `unsafe` code to `xtask`.
The projection keeps the live
file's protected security descriptor, and worker execs are ephemeral, ignore
user configuration and rules, and do not expose Codex home/profile variables to
model-started commands. An armed cleanup removes that one verified link on every
ordinary return and before artifact replay or PASS. A power loss can leave the
additional name until the next owned attempt, and token rotation can make that
name refer to the retired file. Startup therefore removes only the exact direct
regular `codex-home/auth.json` name beneath the verified retained scratch and
never follows a link, recurses through the directory or requires it still to
equal the current live credential. It then refuses to reuse the retained run.
This does not claim a crash-proof credential broker that the pinned CLI does
not provide.
The Python entrypoint is another private wrapper: it accepts only the provider's
three integrity-listed mirrored-script launch forms, normalizes them through a
fixed isolated `-I -S -B -X utf8` bootstrap with an owned empty bytecode-cache
prefix which inserts only that mirrored scripts directory, and refuses
arbitrary flags, modules, commands, arguments or source paths. The complete
adjacent Python-owned runtime tree, excluding unreachable site packages and
bytecode caches, is bounded, hashed before use and measured again after its
last use; Python zip, `._pth` and virtual-environment startup controls are
either bound or absent, and operating-system DLLs remain part of the host TCB.
The tree digest proves one stable observed runtime around the owned execution;
it is not a vendor signature or a claim that a Python installation already
compromised before that observation is pristine.
The adjacent Codex installation is also one explicit runtime TCB: the exact
`codex.exe`, code-mode host, command runner and Windows sandbox setup binaries
are bounded and aggregated under `parent-of-tool.codex`, recorded in the
request and qualification, checked by every private wrapper before and after
child execution, and remeasured after the provider's last use. This observed
four-file identity binds what the installed CLI can launch; it is not a vendor
signature and does not include operating-system libraries.
Two bounded `config/read` dialogues first inventory and then prove the effective
Codex configuration. Custom providers and names outside the pinned CLI's safe
raw key-segment grammar are refused; every ambient MCP route is replaced by a
disabled inert transport beneath private scratch, every ambient plugin is
disabled, notifications and memories are off, and the official ChatGPT route,
model and reasoning are exact. The private CLI wrapper injects that same policy
after provider-supplied options for every worker app-server or exec invocation,
then removes its Codex control environment before starting the hash-pinned real
CLI. Worker exec arguments bind the exact model, source/resume shape and one
owned scan-directory working path; sandbox, extra-directory, cyber-program,
output-schema and image options are refused.
Model-started commands inherit nothing else: their fixed shell environment is
derived from the already qualified Node, Python, Codex and Git parent
directories, the verified Windows `System32`/`cmd.exe`, and the owned temporary
directory. The wrapper reconstructs and rechecks that policy before every exec,
and the live credential probe and MCP server receive the same values.
Immediately before the provider session begins, the pinned CLI runs the current
`xtask` as a sentinel inside that same exact managed read-only sandbox and
network-disabled state. The sentinel requests only Windows `FILE_READ_DATA` on
the projected private `auth.json`, never reads or prints credential bytes, and
must terminate with the one permission-denied code and empty streams. Any
readable credential, different error, output, runtime drift or process failure
blocks the scan.

Each run reserves one exclusive direct child of an OS-temporary Deep root,
with separate `scan-root`, `state` and `temp` children. Provider scratch cannot
be under the selected repository because the coordinator refuses that layout.
The returned scan directory must be the exact new two-level child created under
the private scan root and `manifestPath` must be its direct
`scan-manifest.json`. A throttled no-follow walk fail-stops the Job when sampled
file count, depth or total bytes crosses its limit. This is not a filesystem
quota: writes between observations can overshoot and the retained failed run
can therefore exceed the configured byte threshold. A failed run retains at
most that one directory for diagnosis and blocks another attempt; a successful
run copies only the five fixed artifacts into memory, replays their complete
semantic contract, removes only the exact reserved direct child, and only then
commits the qualification record. Both online evidence construction and offline
verification cross one shared verifier for the owned v6 record; a record
produced by either supported diagnostic/import path cannot bypass it. This is
process provenance
inside the existing operator-account trust boundary, not a signature against a
hostile process already running as the same Windows account or as an
administrator; such interference remains outside the release-evidence threat
model.

Parser tests must accept only the three task-local actions
and reject mixtures, duplicates and the retired seal action. JSON-RPC fixtures
must prove canonical escaping, exact initialize/list/start/complete messages,
notification and ping handling, and refusal of wrong or duplicate IDs, errors,
malformed or oversized lines, premature EOF, incomplete structured results and
any completion content other than the exact pinned one-item text result.
Bounded-process fixtures must prove that a blocked dialogue, noisy streams,
reader failure and early exit stop and reap descendants. Wrapper fixtures must
reject every Python entrypoint and argument outside the three pinned provider
forms, observe the isolated bytecode policy, refuse Python startup controls,
and bind every artifact-server environment value to the owned repository,
scan, wrapper and layout. Owned-run fixtures must reject foreign, nested or
redirected output paths; target, revision, status, warning, scan-ID or
manifest-path drift; hostile environment; changed source, plugin or tools;
scratch-budget excess; and cleanup outside the one reserved child.
Authentication fixtures must prove that credentials are never copied, the held
file permits in-place refresh but cannot be replaced or deleted, a foreign
private-home entry or redirected stale credential is refused, the exact
verified live projection is scrubbed on failure and success, a rotated direct
regular retained name is unlinked without following it, and worker
arguments/environment cannot load or disclose ambient Codex inputs. The live
sandbox preflight must prove exact content-open denial, empty output and
fail-closed handling of every other result. Codex runtime fixtures must bind all
four adjacent executables and reject omission, addition, substitution or drift.
Artifact fixtures must alter only the report bytes and prove that the exact
canonical zero-finding projection, not merely its JSON companions, controls
admission.
Qualification fixtures must prove diagnostic verification
cannot obtain the owned capability, observed bytes are copied and replayed
before commit, forged v6 authority fails both online and offline, and v5/v9
artifacts are invalid. Every new authority predicate receives a retained red
mutation. The real owned Deep run, complete mutation sweep, independent grade,
ordinary gate and evidence replay must all pass on their bound sources before
release publication.

**Rationale.** D-373 removed PASS authority from caller-selected Deep artifact
directories because hashes inside them authenticate bytes, not the process that
produced them. Codex Security 0.1.32 exposes a headless Deep coordinator over
stdio but no signed receipt, and its completed-scan query reads local workbench
state, so importing or re-fetching those bytes would not repair D-373; a live
invocation can, because the release boundary already trusts an exclusive run
under the operator account and `xtask` can own the provider process tree,
dialogue, tools and private output as it owns the cargo-mutants campaign.
Restoring `--seal`, trusting an unsigned provider field, copying workbench
state or adding a nonce fails because the caller that writes the result can
forge or copy each. The plugin's shell wrapper adds ambient runtime selection
and another parser; the provider forbids a scan root inside its target, so
scratch cannot live under `target/`; process-tree ownership is claimed only on
Windows because the bounded runner elsewhere stops only the direct child; and
v5/v9 are retired so the new executable, protocol and authority contract is
not obscured.
