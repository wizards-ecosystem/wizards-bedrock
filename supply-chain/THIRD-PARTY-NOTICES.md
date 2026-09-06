# Third-party notices

The complete machine-readable component list, exact SHA-256 values, selected
licenses and shipped/development scope are in `foundation.spdx`. The source
record is `inventory.txt`.

The shipped image contains these third-party components:

- Limine Rust crate 0.5.0: MIT selected from MIT OR Apache-2.0. Copyright
  (c) 2021 Anhad Singh, 2024 Lysander Mealy.
- bitflags 2.13.1: MIT selected from MIT OR Apache-2.0. Copyright (c) 2014
  The Rust Project Developers.
- noto-sans-mono-bitmap 0.3.2: MIT for crate code, copyright (c) 2022
  Philipp Schuster. Its embedded Noto Sans Mono glyph data is licensed under
  SIL Open Font License 1.1 (`OFL-1.1`), copyright 2022 The Noto Project
  Authors (https://github.com/notofonts/latin-greek-cyrillic). The complete
  font license is in `licenses/OFL-1.1.txt`.
- Limine `BOOTX64.EFI`, inventory version `v10.x-binary-2026-07-18`:
  BSD-2-Clause; the
  upstream copyright and license text are in `boot/limine/LICENSE`.

Development-only crates and artifacts do not reach `wizard-os.img`; they
remain listed in the SBOM because they can affect construction or verification.
Their upstream license conclusions are recorded row by row in the inventory.

Noto is a trademark of Google LLC. This project redistributes only the
pre-rasterized glyph data supplied by the `noto-sans-mono-bitmap` crate and
does not modify or rename the font.
