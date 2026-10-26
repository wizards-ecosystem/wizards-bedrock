# Contributing to Bedrock

Small, scoped contributions are welcome, including reviewed AI-assisted work.
Fork this repository, create a branch in your fork and open a pull request
against `main`. Maintainers review changes before merging; the owner's
direct-to-main workflow does not require outside contributors to obtain access.

Read the relevant [design document](docs/), [code conventions](docs/005-code-conventions.md)
and [finite foundation register](docs/016-pre-wizzy-foundation.md) first.
Explain the defect or improvement and cite a PW row where relevant. Discuss
architecture changes before implementation; decisions belong in the
[decision log](docs/004-decision-log.md). Preserve open rows and stopped triggers.
Bedrock owns the Rust kernel; Realm owns target-side products, and Ink owns
the closed compiler gate. Neither sibling checkout is needed to contribute.

Use the [Windows GNU setup](README.md#building--running) and pinned toolchain.
Documentation and pure-library work needs no VMware, private scanner or sibling
repository. Keep `unsafe` in `wost`; pure hardware-free logic belongs in `libs/`.
Preserve licenses and attribution. Review and understand every submitted change,
including generated changes, explain it in your own words, and avoid bulk PRs
whose behavior and provenance you have not checked. Add no AI co-author trailers.

Run focused real tests for changed logic, extending an existing test where
possible. A new check must be deliberately broken once, observed red, then
restored; machine checks need the existing persistent mutation discipline.
For documentation, the existing decision and prose checks are:

```powershell
cargo test --locked -p xtask --lib decisions::tests::
cargo test --locked -p xtask --lib prose::tests::
```

For a library change, run `cargo test --locked -p PACKAGE` using its actual
package name. Report exact commands, outcomes, host/toolchain, and anything not
run. Explain limitations and include reproduction details for bugs. Do not claim
a compile proves a boot, or an old transcript qualifies current source.

The hosted **Source preview checks** are limited contributor feedback.
Maintainers retain the complete local `cargo xtask ci` gate for substantive
implementation and the stronger qualified-release process in D-425/D-441;
outside contributors need not run multi-hour campaigns for documentation.
D-441's first-launch exception is not a general merge or future-release waiver.

Keep review respectful and technical, assume good faith, and avoid harassment
or private personal information. Maintainers may close abusive or unreviewable
submissions. Use the [security policy](SECURITY.md) for private vulnerability
reports rather than public issues.
