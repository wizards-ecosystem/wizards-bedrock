# D-235 VMware evidence starts from two identified, separate machines

Status: Accepted, 2026-08-29. Advances PW-003 and PW-004.

**Context.** `cargo xtask vmware` could generate only the self-test image, while
PW-004 separately requires the shipped image users boot. Regenerating `vm/`
also reused one machine directory and whatever `serial.log` an earlier round
left there. The task printed the `vmrun` identity but not the exact image
digest, so the two required human observations had no direct command-level
boundary between their state or bytes.

**Decision.** `cargo xtask vmware` remains the self-test default and
`cargo xtask vmware --shipped` is the explicit PW-004 selector. The self-test
machine remains in `vm/`; the shipped machine lives in `vm/shipped/`, so its
NVRAM, logs, disks and Workstation state cannot overwrite the check-bearing
round. Generation truncates that machine's serial transcript before launch and
prints the image kind, byte length, SHA-256 digest, resolved `vmrun` identity
and quoted VMX command. Instructions are image-specific. The keyboard rounds
remain human evidence as D-18 requires; generation does not pretend that
creating a VM booted it.

**Alternatives.** Manually copying a shipped image over the self-test flat disk
was rejected because the command and resulting identity would be implicit.
One shared VM directory was rejected because a stale verdict or NVRAM state can
cross the two evidence rounds. Automating GUI keystrokes through host window
focus was rejected because Workstation exposes no bounded, owned input channel
equivalent to QEMU's QMP route.

**Consequences.** Host checks distinguish both CLI selectors, require separate
machine directories and replace a stale serial fixture with an empty file.
Disabling the selector, merging the directories and appending the transcript
each fail only their named check; mutations 281 through 283 retain the three
defects. The generated image digest and pinned tool identity are now ready to
copy directly into the dated PW-003/PW-004 record. No kernel, unsafe operation,
wire ABI or machine-check count changed.
