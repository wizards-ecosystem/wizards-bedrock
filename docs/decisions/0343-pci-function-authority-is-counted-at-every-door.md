# D-343 PCI function authority is counted at every door

Status: Accepted (2026-09-28), amends D-335; corrects PW-129

**Context.** D-335 recorded that a separate safe duplication door compiled and
failed the exact hostile contract. The contract compiled one call to a method
named `duplicate`, so it caught mutation 1,013 because that mutation used the
same name. A door with any other name compiled with the contract green, and so
did a safe reissue inside `pci-config`, whose private lease is `Copy`; a split
that returns two claims for one; a mutable borrow that retargets a claim; a
static holding a spare claim; and a trait whose default method copies `Self`.

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

**Evidence.** On 2026-09-28 the inventory accepted the current sources, in
which it found four authority-holding types, `FunctionAuthority`,
`FunctionReleaseFailure`, `FunctionClaim` and `ReleaseFailure`, and both
mints. A fixture test presents twenty hostile doors and both mint accounting
failures, and each was refused with its exact diagnostic. Mutations 1,453
through 1,474 each disable one rule of the inventory, and each failed that
fixture test. Mutation 1,013 and mutations 1,475 through 1,481 apply eight
doors to the real sources: the original `duplicate`, a pure reissue, a
hardware split, a pure retarget through `&mut self`, a static spare, a trait
default, a renamed import and an item macro. Each failed the live inventory
with a diagnostic naming its door, and every source was restored byte for
byte.

**Alternatives.** More compiler probes under other method names were rejected
because no list of names is complete. Inspecting only the public methods of
the two authority types was rejected because a door can be a free function, a
trait default, a wrapper's method or a static. Refusing every mention of
authority was rejected because shared references are how configuration writes
prove ownership.
