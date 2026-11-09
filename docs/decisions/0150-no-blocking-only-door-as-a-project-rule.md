# D-150 No blocking-only door, as a project rule

Status: Accepted (2026-08-25) (amended by D-166 and D-181)

**Context.** `013`'s first constraint: every operation that can wait gets a ring
form. The question left open was whether that binds Phase 6 or the project.

**Options.** A phase rule is weaker and gets re-argued. A project rule is
stronger and forecloses a synchronous door somebody may later want for an
operation that only *looks* blocking.

**Decision.** The project. **Every operation that can wait has a ring form, and
no operation is reachable only by a call that blocks.**

The carve-out that makes it liveable already exists and is not an exception:
**a door that cannot wait may stay direct.** At D-150's adoption, `exit`,
`tcb_set` and the clock were the examples. Exit and TcbSet remain direct;
D-166 later removed the clock door in favour of a page load. The rule is about
waiting, not about syscalls.

**Rationale.** The reason to make it a project rule is the reason it would be
re-argued: the first person to want a synchronous `stat` will have a good
local argument, and granting it costs the property that makes the ring worth
having. D-117 caps this machine at sixteen concurrent programs, so
thread-per-outstanding-operation is already wrong here; and any userspace that
multiplexes work onto fewer threads than it has work dies against a
blocking-only door, which is what every language runtime with a scheduler
does.

Stated as a rule rather than a preference because the alternative is not "we
will think about it" but "the second caller decides", and the second caller
will be whoever is in a hurry.

**Consequences.**

- At D-150's adoption, Phase 6 planned `read`, `write` and `open` as
  submissions. The delivered result is narrower: OpenAt and Read are ring
  operations and Write was not built. D-200 later moved Read's VFS and polled
  device work out of `RingEnter` into one bounded worker and made `RingWait`
  use the submission-plus-wait form. D-181 also leaves signals without a
  version-1 representation.
- The rule is checkable at review time and not by machine: nothing can tell a
  door that *cannot* wait from one that merely does not yet.

**Left owed.** The check. A door that can wait is a property of what it calls,
and this project has declined whole-program analysis before for good reasons
(D-147). The honest position is that this is a rule a reviewer applies, and
the entry exists so the reviewer knows to.
