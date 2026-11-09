# D-444 VMware parser regressions retain captured guest records

Status: Accepted (2026-10-26), qualification OPEN; advances PW-003

**Context.** Constructed transcript fixtures can repeat a verifier's mistaken
marker ordering. The retained 2026-10-24 self-test and shipped captures from
`8f4586a` provide independent guest-emitted records, including the self-test
image's historical verdict of 294 passing checks.

**Decision.** Store those exact serial bytes under `xtask/fixtures/vmware/`
with the capture date, source revision and selector in each filename. Mark
`*.serial` binary so checkout cannot rewrite the captured CRLF records. A
private expected-count callback lets the regression fixture retain its original
check count; the public qualifier still derives its expected count from the
current source through `expected_selftest_checks`.

One host witness accepts each capture only for its own selector and rejects
an incorrect historical check count. Mutation 2,443 reverses the required
isolation/APIC order and must fail that witness before restoration. The
captures supply parser regression coverage, not a fresh boot, current-source
qualification or foundation closure. PW-003 and the current machine and
complete-sweep obligations remain open.

**Alternatives.** Rewriting old captures whenever the check count changes
would erase their provenance. Synthetic records alone cannot expose a shared
ordering mistake. Letting production callers choose a count would weaken the
source binding. The private fixture seam preserves the public qualifier's
existing authority.
