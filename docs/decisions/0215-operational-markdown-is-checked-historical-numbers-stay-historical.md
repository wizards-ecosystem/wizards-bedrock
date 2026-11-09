# D-215 Operational Markdown is checked; historical numbers stay historical

Status: Accepted (2026-08-28), amends D-146; closes PW-507

**Context.** D-146 corrected thirteen numeric claims but explicitly stopped
before `008` through `012`. Later prose checks tied two current Phase 6 totals
to source, while every local Markdown path, heading fragment and written
`cargo xtask` invocation remained unchecked. A moved document could leave a
plausible blue link to nowhere, and a renamed task could remain the command a
reader was told to run.

The deferred numeric pass found one actual contradiction. D-76's unsafe
boundary checkpoint had 75 machine checks. D-77 then added four during the
completed-phase audit and states that Phase 4 closed at 79, which is also the
baseline `011` uses. The closing paragraph in `009` nevertheless called the
75-check intermediate point the phase close. The same pass found a prospective
`gdb` xtask formatted as a concrete executable command even though no such task
exists.

**Decision.** The host prose suite now reads every project-authored Markdown
document under `docs/` plus the root, userland and repository-tool guides. A
local link must remain inside the workspace and name an existing path. A
fragment must name a generated heading anchor in the target document. Upstream
bootloader documentation is not rewritten or made part of this project's
promise.

Every `cargo xtask` invocation formatted as inline code or a fenced command is
fed to the same CLI parser that runs the tool. A bare `cargo xtask` remains
legal prose because the docs deliberately use it to describe the error case;
a proposed task is named as a proposal rather than formatted as an executable
command.

`014`'s closing table remains the one current numeric summary. Its machine
count is derived from `EXPECTED_CHECKS`, its mutation total from the registry,
its later-individual count must equal total minus the last full sweep, and its
last decision must equal the decision log's last heading. `016` points to that
checked table instead of copying the changing machine and mutation totals into
three more current-facing cells.

**2026-09-06 enforcement correction.** A later paragraph in `014` still called
the image “current” beside an obsolete 189-check total, even though the closing
table had advanced. The prose check now rejects a current numeric restatement
outside that table while continuing to permit explicitly dated and
“then-current” historical evidence. The same pass makes contributor and README
guidance name both PW-601 prerequisites and the separate VMware transcript
qualification step. Six focused defects remain mutations 1,343 through 1,348.

The manual `008`–`012` pass preserves measurements made at their dated
research and slice checkpoints. Each document now says explicitly that
numbers, tool versions and words such as “current” are scoped to that date;
later correction notes and this log govern the present tree. `009` now
distinguishes D-76's 75-check checkpoint from D-77's 79-check phase close.
This is the distinction D-146 required: correcting a false historical claim
without replacing valid history with today's unrelated total.

**Rationale.** Local target existence, heading identity and CLI grammar are
cheap, deterministic properties with repository-owned sources of truth. They
belong in the ordinary host suite. External URL availability does not: a
network-dependent gate would turn a remote outage into a source failure and
would still not prove that the cited page supports the sentence.

Current counts need one checked restatement, not a growing collection of
copies. Historical measurements need a date and an honest correction, not a
rewrite that makes an old decision appear to have observed future work.

**Alternatives.** A general Markdown package was unnecessary for the current
inline-link and generated-heading surface; the deliberately small reader has
fixtures at the exact forms the repository uses. Pinging every external URL
was rejected as nondeterministic and semantically weak. Replacing all old
totals with 212 was rejected because it would falsify phase evidence rather
than repair it. Maintaining another hand-written list of xtask names was
rejected because the real parser is already the authority.

**Consequences.** Pointing README's architecture link at a missing file and
pointing AGENTS' Development link at a missing heading each failed only
`every_local_markdown_link_and_anchor_resolves`. Renaming the documented
`test-boot` task failed only `every_documented_xtask_invocation_parses`.
Changing `014`'s current machine total failed only
`the_phase_close_says_what_the_source_says`. All four transformations were
restored and persist, bringing the mutation registry to 204. External-source
truth and prose semantics remain reading work; this decision closes the
structural and numeric gap, not the wider reproducibility work in PW-508 and
PW-509.
