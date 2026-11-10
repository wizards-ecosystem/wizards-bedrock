# D-75 Phase 11 is an integration milestone, not a browser

Status: Accepted, 2026-08-23.

**Context.** `000-vision.md` promises the OS will "browse the web", and Phase
11 is "port NetSurf". The 2026-08-23 research checked what it buys.

NetSurf's last release is **3.11, 2023-12-28**. Its documentation says it has
*"only primitive and incomplete support for JavaScript, which we disable by
default"* and that full support is *"a long way off"*. Its engine is Duktape,
ES5.1-era, and `UnimplementedJavascript.md` lists **1,539** missing bindings.
Its baseline is HTML 4.01 and CSS 2.1.

Porting is modest: a new frontend is about 1,000 lines minimal, 7,000 basic,
and the framebuffer frontend has "no particular operating system or GUI
toolkit requirements". Redox ships it.

**Decision.** NetSurf stays the Phase 11 target, described by what it proves:
the libc, windowing system, network stack and TLS work together against a real
third-party codebase. "Browse the web" leaves `000-vision.md`.

**Rationale.** It is a right-sized integration trophy, and it is demonstrably
achievable. It is not web access. A roadmap promising web access while
delivering the trophy reads as failure at the moment it should read as success.

Alternatives weighed. Servo: Redox launched it in October 2025 and reported it
crashes when a second page loads. Ladybird: pre-alpha after 25,000 commits of
funded full-time work, targeting a stable release in 2028, requiring Qt 6.9+,
Skia and Mesa, and closed to outside contributions since June 2026. Nobody solo
has a working browser; SerenityOS needed 421 contributors and then had to spin
its browser out into a funded nonprofit.

**Consequences.**

- `000-vision.md`'s destination list says what NetSurf renders.
- The novel engineering in Phase 11 is coupling NetSurf's `schedule` operation
  to the async ring. Nobody in the surveyed record has done it, so it is
  unknown territory, not a port.
- `links` stays the Phase 9 text-browser milestone; it needs pthreads or fork
  for async DNS; its DOS port runs fully synchronous and still works.
