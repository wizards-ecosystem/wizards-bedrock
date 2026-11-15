# Contributing to Bedrock

Small, scoped contributions are welcome, including reviewed AI-assisted work. Fork the repository,
create a branch in your fork and open a pull request against `main`. A maintainer reviews every
change before it merges.

Read `docs/architecture.md` and `docs/conventions.md` first. Discuss an architecture change in an
issue before implementing it. Keep `unsafe` in `wost` and hardware-free logic in `libs/`. Preserve
licenses and attribution.

Understand every change you submit, including generated code, and explain it in your own words.
Add no AI co-author trailers. Avoid bulk pull requests whose behaviour you have not checked.

Run the checks your change touches: `cargo test --locked -p PACKAGE` for a library, and
`cargo xtask ci` for kernel work if you can. Report the exact commands, their results, your host
and toolchain, and anything you did not run. A compile is not a boot.

Keep review respectful and technical. Report vulnerabilities through [SECURITY.md](SECURITY.md),
not public issues.
