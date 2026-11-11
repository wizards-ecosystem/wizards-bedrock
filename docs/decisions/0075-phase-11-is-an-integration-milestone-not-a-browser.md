# D-75 Phase 11 is an integration milestone, not a browser

Status: Accepted, 2026-08-23.

**Decision.** NetSurf stays the Phase 11 target, described by what it
proves: the libc, windowing system, network stack and TLS work together
against a real third-party codebase. "Browse the web" leaves
`000-vision.md`.

**Rationale.** NetSurf renders HTML 4.01 and CSS 2.1 with JavaScript
disabled by default: a right-sized, achievable integration trophy, not web
access. A roadmap promising web access while delivering the trophy reads as
failure at the moment it should read as success. Servo and Ladybird are not
solo-portable, and no solo project has a working browser.

**Consequences.**

- `000-vision.md`'s destination list says what NetSurf renders.
- The novel engineering in Phase 11 is coupling NetSurf's `schedule`
  operation to the async ring. Nobody in the surveyed record has done it, so
  it is unknown territory, not a port.
- `links` stays the Phase 9 text-browser milestone; it needs pthreads or
  fork for async DNS; its DOS port runs fully synchronous and still works.
