# D-20 Networking plan: smoltcp in-kernel, rustls in userspace

Status: Accepted (planned), 2026-07-18.

**Decision.** The TCP/IP stack is [smoltcp](https://github.com/smoltcp-rs/smoltcp)
(production bare-metal Rust, also Asterinas's choice) integrated
in-kernel behind async `Socket` handles; TLS via rustls in userspace.
Drivers: virtio-net (QEMU) and e1000 (VMware) first.

**Amended by D-183.** `rustls` is a third-party library exception behind the
smallest Wizzy-callable TLS adapter the platform needs; it does not authorize a
first-party Rust TLS service, client or application. Those remain Wizzy. The
project-owned adapter migrates when Wizzy can express the required boundary
without weakening safety or interoperability.

**Known caveat** (research pass): smoltcp lacks TCP congestion control —
fine for LAN/VM development, must be revisited before real-network
workloads are a claim we make.
