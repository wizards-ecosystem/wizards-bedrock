# D-432 A failed PCI observation revokes its snapshot until reset

Status: Accepted (2026-10-23), qualification OPEN; amends D-371 and D-392; advances PW-713

The public topology API previously forgot an observation error. A caller could
ignore the error and seal the remaining structurally valid snapshot, or retain
authority from a previously sealed snapshot after a failed special observation.
The production kernel already propagates every observation error and resets
before its complete census; the review found no current kernel bypass.

Every public observation now first revokes the seal and refuses a retained
error. The first local or delegated observation error is sticky until `reset`;
later observations and `seal` return it, and authority queries remain closed.
The exact-token non-requester and quiesced-requester doors use the same rule.
Structural sealing errors keep their existing semantics. Each function is
observed once per snapshot; successful re-observation is not replacement.

Relying on every caller to remember prior errors leaves a misleading public
contract. Making every sealing error sticky would change unrelated recovery
semantics. One private optional error preserves the existing result type and
requires no allocation or new public operation.

Five public-behavior fixtures cover ordinary errors, both special doors,
matching-token delegated errors, all authority queries, first-error retention
and reset recovery. Their exact retained mutations, pristine/restored runs and
the current complete sweep are required. Static review and registration alone
do not close PW-713.
