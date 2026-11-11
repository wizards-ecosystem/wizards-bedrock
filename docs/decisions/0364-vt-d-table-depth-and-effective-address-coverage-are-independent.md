# D-364 VT-d table depth and effective address coverage are independent

Status: Accepted, 2026-10-07. Amends D-358; advances PW-578.

**Decision.** SAGAW selects the widest supported modelled table depth. Every
mapped byte must remain below both that table's AGAW and the unit's MGAW.
A supported deeper table can serve narrower addresses. MGAW 64 must not
cause a shift by 64. No supported depth still refuses takeover; DMA isolation
and bus-master admission remain mandatory.
Width refusal includes the raw CAP word so another refusal can be diagnosed.

**Rationale.** Two human VMware boots on `96c906d` each passed 210 of 271
checks, and both NVMe candidates refused VT-d width selection; the selector
incorrectly required AGAW to be no greater than MGAW. Intel's
[Capability
register definition](https://edc.intel.com/content/www/us/en/design/publications/13th-generation-core-processor-datasheet-volume-2-of-2/capability-register-cap-reg-0-0-0-vtdbar-offset-8/)
limits addressability to the minimum of MGAW and the supported table's AGAW.
The failing unit's raw CAP word was not logged; its exact tuple is unproved.

**Alternatives.** Refusing a supported deeper table rejects a usable unit.
Dropping MGAW without bounding mappings overstates the domain's reach.
A VMware exception would hide the same error on other conforming units.

**Consequences.** The failed human boots close no hardware row. Retained
mutations 1,603 through 1,605 restore the old selector or bypass each coverage
limit, and mutation 1,623 reverts the production mapping seam to geometric
coverage alone; each fails its named witness or AST source contract.
