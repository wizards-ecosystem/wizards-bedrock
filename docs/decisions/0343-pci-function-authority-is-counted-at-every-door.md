# D-343 PCI function authority is counted at every door

Status: Accepted, 2026-09-28. Amends D-335; corrects PW-129.

**Decision.** One syntax inventory covers every item outside test code in
`libs/pci-config/src` and `wost/src`, the two crates that can construct
function authority. It starts from `FunctionAuthority` and `FunctionClaim` and
adds each struct, enum, union, type alias and renamed import that holds one by
value until the set stops growing. It then requires:

- no function signature produces more owned authority than it consumes by
  value, except the audited mints `FunctionClaims::claim` and
  `claim_function`, each seen exactly once;
- parameters and fields hold authority only by value or by shared reference,
  and a mutable borrow, raw pointer, closure, trait object or generic bound
  that names authority is refused;
- no static, constant or associated type holds authority;
- a type holding authority implements only `Debug`, `Display`, `Error`,
  `PartialEq` and `Eq`, and derives only `Debug`, `PartialEq` and `Eq`;
- no macro receives an authority name, and a file that names authority
  contains no item-position macro, because syntax cannot see what a macro
  expands to.

`Option` and `Result` count as alternatives, tuples and fixed arrays as
products, and any other container as unbounded. The name-only compiler probe
is removed, and the inventory now witnesses mutation 1,013.

A generic function that manufactures a `T` from `&T` needs `unsafe`, which
D-41's inventory confines to reviewed single-operation blocks in `wost`. The
type inventory leaves that case to it.

**Rationale.** D-335's hostile contract compiled one call to a method named
`duplicate`, so it caught mutation 1,013 only because that mutation used the
same name. A door under any other name compiled with the contract green, as
did a safe reissue inside `pci-config`, whose private lease is `Copy`, a split
returning two claims for one, a retargeting mutable borrow, a static spare
claim and a trait default that copies `Self`. Mutations 1,453 through 1,481
retain one break of each inventory rule and seven further doors on the real
sources.

**Alternatives.** More compiler probes under other method names were rejected
because no list of names is complete. Inspecting only the public methods of
the two authority types was rejected because a door can be a free function, a
trait default, a wrapper's method or a static. Refusing every mention of
authority was rejected because shared references are how configuration writes
prove ownership.
