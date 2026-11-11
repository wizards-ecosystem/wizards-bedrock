# D-178 Ask the device what it is, not what it is set to

Status: Accepted, 2026-08-25.

**Decision.** Ask the keyboard who it is. On VMware the set query answered
`0x41`, not `2`, while the controller's configuration byte said translation was
off. If bytes reaching the decoder are translated into set 1 while the decoder
reads set 2, the keyboard types the wrong characters, and only the checks that
need a human would notice.

An MF2 keyboard answers `0xF2` with `0xAB 0x83`. The same keyboard behind a
translating controller answers `0xAB 0x41`, because translation rewrites that
reply exactly as it rewrites the scancodes, which is why Linux's `atkbd` reads
the pair rather than trusting the configuration bit. The pair is a question
about **the path the bytes take**, answerable with no human, where the set query
and the configuration byte ask what something was *told* to be.

**The rule:**

> **A device's configuration says what it was asked for; its identity says what
> it is.** Where a check depends on the second, read the second. A controller
> that reports its own translation bit is reporting a value it was written,
> and the byte that arrives at the decoder is the only evidence of what it
> actually does.

**Rationale.** Three checks stand between the decoder and a wrong-set keyboard,
failing in different places on purpose: the configuration byte catches a
controller that was never configured, the set query a keyboard that was never
switched, and the identity a controller that answers both correctly and
rewrites the bytes anyway. Only the third is immune to being lied to. `0x41`
remains unexplained as a scancode-set reply; this makes its consequence
observable without a human.

**Consequences.**

- `Controller::identity`, `input::identity`, and the check *the keyboard
  identifies as one the decoder reads raw*.
- The self-test names both the set and the identity when either is not what it
  expects, so a machine that disagrees says how in one boot.
