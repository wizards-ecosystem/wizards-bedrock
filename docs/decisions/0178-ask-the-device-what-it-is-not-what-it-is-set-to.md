# D-178 Ask the device what it is, not what it is set to

Status: Accepted (2026-08-25)

**Context.** With D-177's probes fixed, the VMware boot reached
`passed=177 failed=5`. Four of the five need a human at the machine to press a
key. The fifth does not:

```
[warn ] keyboard: reports scancode set Some(65), not 2
[error] selftest: the keyboard is speaking scancode set two FAILED
```

That warning is D-176's rule paying for itself two commits after it was
written: the check had known only that the answer was not two, and one boot's
worth of extra prose turned an unexplained red into a number to reason about.

**`0x41` is not obviously anything.** It is not `2`. The controller's
configuration byte says translation is *off* — the neighbouring check reads
that bit and passes. So either the controller reports its own configuration
falsely, or its answer to *this particular query* is rewritten, or it answers
the query with something else entirely.

The distinction is not academic. If bytes reaching the decoder are translated
into set 1 while the decoder reads set 2, the keyboard types the wrong
characters, and the only checks that would notice are the four that need a
human. A property this kernel depends on would be resting on a keypress.

**Decision.** Ask the keyboard who it is.

An MF2 keyboard answers `0xF2` with `0xAB 0x83`. The same keyboard behind a
translating controller answers `0xAB 0x41`, because translation rewrites that
reply exactly as it rewrites the scancodes — which is why Linux's `atkbd` reads
the pair rather than trusting the configuration bit. The pair is therefore a
question about **the path the bytes take**, answerable with no human, where the
set query and the configuration byte are both questions about what something
was *told* to be.

**The rule:**

> **A device's configuration says what it was asked for; its identity says what
> it is.** Where a check depends on the second, read the second. A controller
> that reports its own translation bit is reporting a value it was written,
> and the byte that arrives at the decoder is the only evidence of what it
> actually does.

**Rationale.** Three checks now stand between the decoder and a wrong-set
keyboard, and they fail in different places on purpose: the configuration byte
catches a controller that was never configured, the set query catches a
keyboard that was never switched, and the identity catches a controller that
answers both correctly and rewrites the bytes anyway. Only the third is immune
to being lied to, and only the third was missing.

`0x41` remains unexplained as a scancode-set reply, and this entry does not
pretend otherwise. What it does is make the *consequence* of that reply
observable without a human, so the next boot says whether the number matters or
is merely strange.

**Consequences.**

- `Controller::identity`, `input::identity`, and the check *the keyboard
  identifies as one the decoder reads raw*. 183 machine checks, mutation 67.
- The self-test now names both the set and the identity when either is not
  what it expects, so a machine that disagrees says how in one boot.
- Phase 6's VMware close still needs one boot with a human at it: four
  keystroke checks cannot run headless, and whether the fifth is a VMware
  quirk or a real mis-decode is a question the identity answers and the
  keystrokes confirm.
