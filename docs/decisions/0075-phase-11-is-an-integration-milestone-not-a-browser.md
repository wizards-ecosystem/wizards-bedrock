# D-75 Phase 11 is an integration milestone, not a browser

Status: Accepted, 2026-08-23.

**Context.** `000-vision.md` promises the OS will "browse the web", and Phase 11
is "port NetSurf". The 2026-08-23 research checked what that buys.

NetSurf's last release is **3.11, 2023-12-28**. Its own documentation says it
has *"only primitive and incomplete support for JavaScript, which we disable by
default"* and that full support is *"a long way off"*; its engine is Duktape,
ES5.1-era, and `UnimplementedJavascript.md` lists **1,539** missing bindings.
Its baseline is HTML 4.01 and CSS 2.1.

The porting cost is genuinely modest — a new frontend is about 1,000 lines
minimal, 7,000 basic, and the framebuffer frontend has "no particular operating
system or GUI toolkit requirements". Redox ships it.

**Decision.** NetSurf stays the Phase 11 target. The milestone is described by
what it proves — that the libc, the windowing system, the network stack and TLS
work together against a real third-party codebase — and "browse the web" comes
out of `000-vision.md`.

**Rationale.** It is a good, right-sized integration trophy and it is
demonstrably achievable; it is not web access, and a roadmap that promises the
second while delivering the first will be read as a failure at exactly the
moment it should be read as a success.

The alternatives were weighed. Servo: Redox launched it in October 2025 and
reported it crashes when a second page loads. Ladybird: pre-alpha after 25,000
commits of funded full-time work, targeting a stable release in 2028, requiring
Qt 6.9+, Skia and Mesa, and closed to outside contributions since June 2026.
Nobody solo has a working browser; SerenityOS needed 421 contributors and then
had to spin its browser out into a funded nonprofit.

**Consequences.**

- `000-vision.md`'s destination list says what NetSurf renders.
- The genuinely novel engineering in Phase 11 is coupling NetSurf's `schedule`
  operation to the async ring. Nobody in the surveyed record has done that
  combination, so it is unknown territory rather than a port.
- `links` stays the Phase 9 text-browser milestone. It needs pthreads or fork
  for its async DNS; its DOS port runs fully synchronous and still works.
