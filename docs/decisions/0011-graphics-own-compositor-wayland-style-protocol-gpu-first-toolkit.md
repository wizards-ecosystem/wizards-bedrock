# D-11 Graphics: own compositor, Wayland-style protocol, GPU-first toolkit

Status: Accepted, 2026-07-18.

**Context.** Isaac wants his own windowing system, "probably Wayland
style, but maybe something way more custom."

**Decision.** Own compositor and window protocol in the Wayland model —
clients render into shared `MemoryObject` buffers, the compositor
composites; protocol details free to diverge from Wayland where we have
better ideas. Native widget toolkit renders GPU-first (wgpu/vello-style
compute 2D) targeting virtio-gpu in VMs, dumb-framebuffer fallback.
NetSurf gets a toolkit/framebuffer backend when its phase arrives.

**Rationale.** Wayland's buffer-sharing model is the proven 2026 shape
and maps directly onto our memory-object handles; GPU-compute 2D is
where rendering is headed.

**Consequences.** GUI phase depends on memory objects + channels being
solid; a terminal emulator is the first GUI client.

**Amended by D-318 and D-321.** The shell model over this compositor is
the strip with snap semantics under compositor-drawn chrome (D-318), and the
toolkit is GPU-first with a CPU fallback, drawing from one token document
(D-321).
