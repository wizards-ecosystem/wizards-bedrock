# D-297 The population run attests the advisory database; the pinned audit is bound by Git

Status: Accepted, 2026-09-06. Amends D-274; advances PW-028.

**Decision.** The two runs keep their roles and both are recorded. The
population run's report is the tool's attestation: its `last-commit` must
equal the Git revision captured after it and its `last-updated` must be an
RFC3339 time, which the record repeats as `database.last-updated`. The
pinned run's report is the verdict; its database identity may be null, the
shape `--no-fetch` produces, or the exact revision, and it must count the
same advisories the population run saw. The verdict is bound to the revision
by the checkout identity proven identical before and after it, exactly as
D-274 already required. Both transcripts are artifacts of the record and of
the evidence bundle, whose schema becomes v6 because its file set grew.
Offline replay verifies the population transcript as fetched, the pinned
transcript as pinned, their agreement, and the record's repeated time.

**Rationale.** D-274 required the revision-pinned `--no-fetch` report to name
the exact database revision, but `cargo-audit` 0.22.2 learns a database commit
only from the fetch it performs: the same tool against the same checkout
reports `"last-commit":"5a0e…"` when it fetches and `"last-commit":null,
"last-updated":null` under `--no-fetch`. D-274's pinned-run expectation could
never be met by the tool it names, and the first real `cargo xtask
qualify-advisory` failed with "cargo-audit `last-commit` is not a string".

**Alternatives.** Dropping `--no-fetch` from the pinned run was rejected
because the verdict would again run against a database the tool may change
while it works, the defect D-274 removed. Accepting a null identity in a
fetched report was rejected because it would discard the one attestation the
tool can make. Keeping only the population run was rejected because the
verdict would then have no run whose inputs were frozen and re-verified
around it.

**Proof.** A unit fixture shows the null identity is accepted only under the
pinned binding; replay fixtures refuse a population transcript without
identity, a pinned audit that saw a different advisory count, and a record
whose repeated update time is malformed or differs from the attested one.
Four manual breaks each reddened only their fixture before restoration and
remain mutations 1,354 through 1,357; the three D-274 report mutations whose
anchors moved into the binding match were re-anchored and re-proven.
