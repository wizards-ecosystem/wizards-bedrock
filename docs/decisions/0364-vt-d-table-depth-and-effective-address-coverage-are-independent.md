# D-364 VT-d table depth and effective address coverage are independent

Status: Accepted (2026-10-07), amends D-358; advances PW-578

**Context.** Two human VMware boots on `96c906d` each passed 210 of 271
checks. Both NVMe candidates refused VT-d width selection. The selector
incorrectly requires AGAW to be no greater than MGAW. Intel's [Capability
register definition](https://edc.intel.com/content/www/us/en/design/publications/13th-generation-core-processor-datasheet-volume-2-of-2/capability-register-cap-reg-0-0-0-vtdbar-offset-8/)
limits addressability to the minimum of MGAW and the supported table's AGAW.
The failing unit's raw CAP word was not logged; its exact tuple is unproved.

**Decision.** SAGAW selects the widest supported modelled table depth. Every
mapped byte must remain below both that table's AGAW and the unit's MGAW.
A supported deeper table can serve narrower addresses. MGAW 64 must not
cause a shift by 64. No supported depth still refuses takeover; DMA isolation
and bus-master admission remain mandatory.
Width refusal includes the raw CAP word so another refusal can be diagnosed.

**Proof required.** Exhaustive host SAGAW/MGAW and address-boundary checks,
retained mutations restoring the old selector and bypassing each coverage
limit, release/debug boots and CI, then fresh VMware storage/fault witnesses.
The failed human boots close no hardware row.

**Alternatives.** Refusing a supported deeper table rejects a usable unit.
Dropping MGAW without bounding mappings overstates the domain's reach.
A VMware exception would hide the same error on other conforming units.

**Host evidence.** On 2026-10-07 the 33 VT-d host checks and doctest passed.
Mutations 1,603 through 1,605 each failed the selected selection or effective
coverage witness with exit 101, then restored source byte for byte. Release,
debug boots each passed 271/271 on the repair tree. The ordinary gate and the
new VMware observation remain required.
Mutation 1,623 reverted the production mapping seam to geometric coverage
alone and failed its AST source contract before exact restoration.
