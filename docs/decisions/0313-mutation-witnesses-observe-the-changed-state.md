# D-313 Mutation witnesses observe the changed state

Status: Accepted (2026-09-12), amends D-28, D-48, D-126, D-239, D-257, D-276 and D-301; advances PW-001

**Context.** The last six survivors mixed redundant defenses with witnesses
which observed a different transaction phase. Fabricating user-copy `CS` still
left the ring-3 page-fault bit to reject recovery. Cursor removal deleted both
hide and show, but scrolling cleared the old mark and the check never required
a new one. Translation's later deliberate wrong-numbering seed inverted a stale
decoder into the expected one. A one-word-short framebuffer shadow never
accessed its terminal word during the boot. The I/O-APIC fixture ignored the
initial mask write, so the first readback refused before the mutated final
destination-clear check. The APIC failure observer never executed the successful
commit whose Ready/interrupt-restoration order had changed.

**Decision.** Bind the saved code selector through the existing exact
architectural-adapter source contract. Make the cursor mutation remove only the
new cursor publication and require both that the old mark is absent and the new
underline is visible. Check decoder/identity agreement immediately after the
translation transaction, before deliberately perturbing the decoder. Define a
complete framebuffer shadow as an exact nonempty word count. Add an I/O-APIC
takeover case which accepts the mask and ignores only the destination clear.
Require the successful APIC commit's Release publication to precede interrupt
restoration in the existing ordered-publication contract.

Forcing unrelated machine paths to reach a terminal framebuffer word or turning
a successful APIC commit into a failure probe was rejected because those would
test incidental workload shape. Removing the independent page-fault privilege
or first I/O-APIC readback defenses was rejected because witness repair must not
weaken the boundary it proves.

**Proof.** The exact user-copy and APIC-order mutations failed
`each_recovery_adapter_binds_the_live_architectural_values` and
`apic_ready_publication_precedes_dependent_vector_reads`. Four release QEMU
breaks failed exactly `a scroll moves one visible cursor`, `translation updates
the decoder from the controller identity`, `the console has a complete shadow
to scroll from`, and `a refused keyboard route restores configuration, masking
and mappings`, each at 260/261 with no second failure. After restoration the
combined release image passed 261/261. Together D-302 through D-313 now repair
all 34 survivors in the `f28428a` report; its 24 inconclusive outcomes and the
clean complete successor sweep remain PW-001.
