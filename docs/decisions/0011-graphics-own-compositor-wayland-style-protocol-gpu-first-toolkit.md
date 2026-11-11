# D-11 Graphics: own compositor, Wayland-style protocol, GPU-first toolkit

Status: Accepted, 2026-07-18.

**Decision.** Own compositor and window protocol in the Wayland model:
clients render into shared `MemoryObject` buffers and the compositor
composites. Protocol details may diverge from Wayland where we have better
ideas. The native widget toolkit renders GPU-first (wgpu/vello-style compute
2D), targeting virtio-gpu in VMs with a dumb-framebuffer fallback. NetSurf
gets a toolkit/framebuffer backend when its phase arrives.

**Rationale.** Wayland's buffer sharing maps directly onto our memory-object
handles; GPU-compute 2D is where rendering is headed.

**Consequences.** The GUI phase needs solid memory objects and channels; a
terminal emulator is the first GUI client.

**Amended by D-318 and D-321.** The shell model over this compositor is the
strip with snap semantics under compositor-drawn chrome (D-318). The toolkit
is GPU-first with a CPU fallback, drawing from one token document (D-321).
