# D-313 Mutation witnesses observe the changed state

Status: Accepted, 2026-09-12. Amends D-28, D-48, D-126, D-239, D-257, D-276 and D-301; advances PW-001.

**Decision.** Bind the saved code selector through the existing exact
architectural-adapter source contract. Make the cursor mutation remove only the
new cursor publication and require both that the old mark is absent and the new
underline is visible. Check decoder/identity agreement immediately after the
translation transaction, before deliberately perturbing the decoder. Define a
complete framebuffer shadow as an exact nonempty word count. Add an I/O-APIC
takeover case which accepts the mask and ignores only the destination clear.
Require the successful APIC commit's Release publication to precede interrupt
restoration in the existing ordered-publication contract, which mutation 849
breaks by restoring interrupts before that publication.

With this record D-302 through D-313 repair every survivor in the `f28428a`
report; its inconclusive outcomes and the clean complete successor sweep remain
PW-001.

**Rationale.** The last six survivors mixed redundant defenses with witnesses
which observed a different transaction phase: the ring-3 page-fault bit still
rejected a fabricated `CS`, scrolling cleared the old cursor, a later seed
inverted a stale decoder, the boot never reached a short shadow's last word,
the I/O-APIC fixture refused at its first readback, and the APIC observer never
ran a successful commit.

**Alternatives.** Forcing unrelated machine paths to reach a terminal
framebuffer word or turning a successful APIC commit into a failure probe was
rejected because those would test incidental workload shape. Removing the
independent page-fault privilege or first I/O-APIC readback defenses was
rejected because witness repair must not weaken the boundary it proves.
