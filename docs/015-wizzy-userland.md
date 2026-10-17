# 015 - Bedrock's versioned userland handoff

D-366 transfers compiler readiness and the Phase 7 product plan to
[Realm](https://github.com/wizards-ecosystem/wizards-realm/blob/main/docs/003-compiler-readiness.md).
D-183 keeps the kernel Rust and first-party target-side applications Wizzy.
D-184's [016-pre-wizzy-foundation.md](016-pre-wizzy-foundation.md) is the
authoritative finite kernel foundation record. This document owns Bedrock's
kernel integration requirements, not Realm's application semantics or Ink's
language semantics.

## Current integration admission

| Contract | Current value |
|---|---|
| First-party target language | Wizzy |
| Compiler-readiness gate | **CLOSED** |

The table records the last admitted compiler-readiness evidence for Bedrock,
not a second authority that can open Realm's gate. Phase 7 product implementation is stopped
and belongs to Realm. Opening Realm's gate does not alone admit an artifact to
Bedrock. A kernel integration ADR must pin the reachable Realm decision and
Ink compiler/runtime/artifact revisions and digests, close the relevant kernel
foundation prerequisites, and prove the kernel ABI contract on the exact image.
Until then the conservative CLOSED observation remains valid independently of
any sibling checkout. PW-601 still requires both a pinned draft artifact and
runtime seam before consumer scaffolding; WZ-001 through WZ-004 belong to Ink.

## Kernel contract

Bedrock owns entry state, calls, rings, handle rights, status values, image
loading, memory protection, kernel process lifecycle, drivers and qualification.
Realm owns launch policy, runtime adapters, services, terminal, shell, desktop,
compatibility ports and composed OS images. Ink owns execution and language
semantics. No compiler, parser, evaluator or language scheduler enters the kernel.

D-226's route-neutral envelope remains `wizard-wizzy-handoff-v1`, targeting
`x86_64-unknown-wizard` with aggregate ABI `wizard-platform-v1`. These are
versioned compatibility identifiers and are unchanged by the product rename.
The aggregate binds entry, ring and time page v1, checked against the native C
transcription at `fixtures/abi-pressure/abi/wizard_abi.h`. The transcription is
checked evidence, not an independent ABI source. Kernel source and its design
contracts remain authoritative. An ABI change requires a versioned decision.

Artifact, runtime and effect versions are exact-match producer-owned tokens.
The envelope binds source, compiler and runtime revisions, dependency-lock
digest, artifact byte length and SHA-256. Sorted effect requirements are
requests, never grants. Realm's launcher grants authority; Bedrock enforces it.
The synthetic `handoff/conformance-v1.wzh` and adjacent opaque artifact prove
the envelope only. They do not pin or execute a real Wizzy program.

The kernel diagnostic image's emitted ring-3 machine code tests the loader and
ABI. It is not Realm, a language runtime, init or a fallback userland. Likewise
the uncompiled POSIX pressure test is retained solely as kernel ABI evidence.
Future runtime and POSIX adapter implementations belong to Realm.

## Independent inputs and proof

An ordinary Bedrock build and `cargo xtask ci` consume only this repository's
committed or vendored inputs. They do not read Realm or Ink working trees and
do not fetch their moving branches. Realm's corresponding integration consumes
explicit immutable Bedrock and Ink inputs. Each backend must independently
prove application-facing behavior and state its isolation limits.

A future integration qualification regenerates a real artifact twice using
pinned compiler/runtime source and locked dependencies, requires identical
bytes, and compares those bytes with the image actually booted. Neither a
format fixture nor a green kernel diagnostic boot substitutes for that proof.
D-201's NX/time admission, D-250's affirmative CPU isolation evidence, W^X,
rights attenuation and DMA isolation remain mandatory.

Existing missing user operations remain register-owned triggers with named
callers. The detailed target-side plan is now Realm's specification; it does
not authorize speculative kernel APIs. Kernel qualifications continue to bind
the exact Bedrock source commit and its permitted closing child under D-330.
Historical qualifications at the old identity never qualify renamed bytes.
