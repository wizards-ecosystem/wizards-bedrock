# D-277 Interrupt sources share one typed global vector assignment

Status: Accepted, 2026-09-04. Amends D-33, D-49, D-81, D-96, D-123, D-205 and D-258; advances PW-535.

**Decision.** `x86-apic` owns the sole assignable-vector type. Its constructor
excludes the CPU's 0 through 31 vectors and the remapped 8259 range 0x20 through
0x2f. A const-generic assignment validates every member and pair before
producing typed values. `kernel::arch::vector` contains the sole production
assignment for the keyboard, syscall, spurious, error, self-interrupt and timer
sources. IDT, local-APIC and I/O-APIC programming consume those typed values;
raw bytes remain only hardware observations decoded at the boundary.

**Rationale.** I/O-APIC routing still accepted a raw byte, local-APIC policy
admitted vectors 16 through 31, and the keyboard, syscall and four local
sources were constants checked only in separate subsets. A duplicate or
reserved value could therefore select an exception-frame stub or dispatch one
source as another through safe code.

**Alternatives and proof obligation.** Retaining separate APIC and IDT guards
was rejected because it leaves uniqueness between subsystems unstated. A
kernel-private newtype was rejected because privileged framework doors could
still accept raw values from another caller. The `x86-apic` tests exhaust all
256 inputs and 65,536 pairs; compiler/source contracts reject raw programming
doors and a second production assignment. Architectural-boundary, remapped-PIC
and duplicate breaks fail only their exact host witnesses. The raw-door and
fake-route breaks are retained in the mutation registry but still need exact
red executions, so PW-535 remains open rather than treating registration as
qualification.
