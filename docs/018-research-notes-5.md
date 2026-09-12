# 018 - Deep Research V: the userland

> **Historical snapshot.** Version numbers, dates, adoption figures and words such
> as "current" or "today" are observations from the production date below. The
> decision log governs the present repository; `019-userland.md` and
> `020-style-guide.md` carry the design this pass informed.

Produced 2026-09-12, before any userland code exists and while the Wizzy
compiler-readiness gate in `015-wizzy-userland.md` is **CLOSED** (D-183). The
pass exists because no earlier research document covers the userland: `003`,
`008`, `010` and `012` are kernel architecture, Rust practice, foundation
verification and storage. Their one userland-shaped conclusion still stands and
is used throughout: `010` section 5's "conform at the boundary, innovate inside".

Four research angles ran in parallel on the same day, each instructed to fetch
primary sources (release notes, specifications, vendor documentation, papers)
and to say when a claim rests on secondary coverage. Nothing in this pass was
measured on this machine; every number is a citation. Citations are marked
`[L-n]` (Linux desktops), `[W-n]` (Windows and macOS), `[A-n]` (alternative and
research systems) and `[T-n]` (shells, terminals, toolkits, text, packaging,
accessibility), with one source list per section. Product names follow
`.github/BRAND.md`: The Wizard's OS is named in full on first mention and as the
OS afterwards.

The brief the angles worked to: a keyboard-first desktop for one developer who
likes simplicity and power, Windows-leaning in convention, written almost
entirely in Wizzy, on a kernel with capability handles, no ambient authority, no
filesystem root, spawn without fork, and an asynchronous completion ring. The
brief asked for creative and untested patterns, not a copy of an existing
desktop.

---

## 0. The questions, and the short answers

| Question | Answer | Confidence |
|---|---|---|
| Window model | A scrollable strip of columns and stacks, with Windows snap fractions and a complete taskbar | High: the strip is the first tiling model that never resizes what is being read (section 1), and the taskbar's restoration history shows what cannot be left out (section 2) |
| Command surface | One command registry feeding menus, a palette, the shell, scripts, accessibility and tests | High: every platform that split these is now unifying them (sections 2 and 4) |
| Shell | Typed pipes over explicit channels by default, bytes as the explicit escape hatch, no macro processing | High on the shape; the wire format must be defined before the shell (section 4) |
| Terminal | A record-based session protocol; VT100 only in a compatibility component | High: the successful terminals are all extending VT toward records; the critics are right about the cause (section 4) |
| Authority UX | Designation is authorization; compositor-drawn trust labels; no permission prompts | High: three independent traditions converged on it (section 3) |
| Toolkit and style | One reactive toolkit, one style source, tokens as an OS API, three independent preferences | High: every desktop that grew a second one is paying for it (sections 1, 2, 5) |
| Text | One shaping and layout stack shared by terminal, editor and toolkit; OFL faces only | High (section 5) |
| Packaging | Content-addressed, reproducible artifacts; generations; A/B images; a safe mode | High (sections 1, 3, 6) |
| Services | A static, verified, supervised, lazily started component graph that is also the admin view | Medium: the combination is not shipped anywhere; the parts are (sections 3, 6) |
| Files | Places instead of a root; queries as documents later; content routing rules | Medium: proven in Haiku and Plan 9, unproven with a capability namespace (section 3) |
| Accessibility | A push-model tree produced by the toolkit and held by the compositor | High: Linux is spending years retrofitting it (sections 1, 7) |

---

## 1. The Linux desktop, 2025 to 2026

### 1.1 GNOME 48 to 50

GNOME 48 (March 2025) shipped notification stacking per application, dynamic
triple buffering, first system-level HDR, battery charge limiting, new Adwaita
Sans and Mono faces derived from Inter and Iosevka, screen-time tracking with a
greyscale nudge, a global-shortcuts portal with permission prompts and centred
window placement by default [L-4]. GNOME 49 (September 2025) replaced Totem and
Evince with GTK4 rewrites, moved Do Not Disturb into Quick Settings, sandboxed
image decoding, and disabled the X11 session by default [L-3][L-5]. GNOME 50
(March 2026) removed X11 entirely, turned variable refresh and fractional
scaling on by default, added a cursor that runs at the monitor's refresh rate
independent of application frame rate, a Reduced Motion setting, an Orca
overhaul, hardware-accelerated remote desktop, parental screen-time limits and a
Files rewrite [L-1][L-2].

The Activities overview is one overlay that is window picker, workspace strip,
application grid and search at once; workspaces are created as they fill and
destroyed when empty [L-23]. The libadwaita language is a small vocabulary
(header bars, boxed lists, view switchers, bottom sheets, adaptive dialogs), and
1.7 to 1.9 added toggle groups, wrap boxes, CSS variables for the document and
monospace fonts, media queries, a shortcuts dialog and a searchable sidebar
[L-8][L-9][L-10]. The HIG's four principles are design for people, make it
simple, reduce effort, be considerate [L-11].

The stance against a tray and desktop icons is coherent (persistent icons are
mostly leaked implementation state) and the replacement is thinner than what it
replaced: the maintainer answer in 2025 is still the Background portal's status
string, notifications, or an extension [L-14][L-15]. Session restore is being
rebuilt as a portal plus compositor tokens and missed GNOME 51 because "the
larger obstacle is GTK and the application-facing API" [L-21]. Mutter still
implements neither layer-shell nor data-control, which is why GNOME has no
clipboard history and third-party panels need extensions [L-70][L-71][L-101].
GNOME OS is image-based with systemd-sysupdate and sysext, and gained a safe
mode that boots with every extension disabled [L-17][L-22][L-110].

### 1.2 KDE Plasma 6.3 to 6.8

Plasma 6.0 set the baseline: Wayland default, a floating panel, a merged
Overview, per-screen ICC, a flatter System Settings [L-24]. The releases since
read as a catalogue of what a shell owes its users: pixel-grid fractional
scaling, panel cloning, Discover flagging permission changes on update (6.3)
[L-25]; per-desktop tiling layouts, an HDR calibration wizard, auto Do Not
Disturb in fullscreen with a summary afterwards, an Xwayland input-snooping
switch (6.4) [L-26][L-35]; rounded corners, scheduled light and dark pairs for
themes and wallpapers, clipboard pinning, an Application Permissions page that
grew from Flatpak-only to portal-level control (6.5) [L-27][L-36]; a first-run
setup split from the installer, a new on-screen keyboard, an optional Plasma
Login Manager, standardised reduced motion (6.6) [L-28][L-112]; per-screen
virtual desktops "after 21 years", type-ahead on the desktop, Union as a
preview, Background Apps in the tray (6.7) [L-29]; Union extended to QtWidgets
through about 9,000 lines of QStyle code (6.8 beta) [L-30].

Union exists because KDE accumulated three incompatible theming paths (Plasma
SVG themes, QtQuick styles, QStyle) and needed one CSS-like source to drive all
of them [L-37][L-30]. It is the clearest lesson in this pass: theming debt from
three generations took years to unify. Activities and virtual desktops "blur
together" for users and starve maintainers [L-45]. The Plasma Login Manager
rationale is that a login greeter with its own theme engine has to reimplement
keyboard layout, Wi-Fi, Bluetooth, brightness and accessibility; the replacement
reuses the desktop session's own startup path [L-38][L-39]. KDE Linux is the
project's reference immutable platform: read-only `/usr`, atomic updates with
rollback entries, Flatpak and containers for everything else; its critics note a
day's delay behind Arch for fixes, no inventory of installed software, and one
upgrade that offered to boot a non-existent image [L-40][L-41][L-42]. 79% of
Plasma 6 users were on Wayland at the end of 2025 and the X11 session ends with
6.8 [L-33][L-31].

### 1.3 COSMIC

COSMIC 1.0 shipped 11 December 2025 on a Smithay compositor with an iced-based
toolkit used for applications and applets alike [L-51][L-111][L-54]. Tiling is a
per-workspace mode rather than a lifestyle; a stack is a compositor primitive;
workspaces are per-output by default; configuration is RON files praised for
reproducibility [L-53][L-55]. The most detailed daily-driver review measured
about 880 MB idle against GNOME's 1.35 GB and around 12 ms input latency
[L-55]. CJK input was broken at launch because the input-method daemons and the
compositor spoke different protocol versions [L-58].

### 1.4 Scrollable and tiling compositors, and the protocol layer

niri lays each monitor out as an infinite horizontal strip of columns; "opening
a new window never causes existing windows to resize"; workspaces stack
vertically per monitor; preset column widths (a third, a half, two thirds)
cycle; tabs group windows in a column; a three-finger swipe scrolls the strip
[L-59][L-61]. Its 2025 and 2026 releases added an MRU switcher with live
previews, true maximise, per-output layouts, screen-reader announcements and
background blur [L-60]. Hyprland moved its configuration from hyprlang to Lua
and users complain that "each new 0.5x version changes everything" [L-62][L-64].
river 0.4 moved every placement, focus and decoration decision out of the
compositor into an external window manager over a protocol it declares stable
[L-66].

Protocol status in September 2026: stable xdg-shell, viewporter,
presentation-time, tablet, linux-dmabuf; staging session-lock, idle-notify,
fractional-scale, cursor-shape, foreign-toplevel-list, security-context,
xdg-activation, toplevel drag and icon and tag, xdg-dialog, image capture,
data-control, workspace, colour management, background-effect; experimental
input-method v2, text-input, session-management, keyboard-filter [L-68]. Colour
management merged after five years and more than 800 review comments [L-73];
session management merged March 2026 [L-21]. The security model is the part
nobody wants back: no client reads another's input or pixels, capture goes
through the portal, and security-context lets a sandbox tag a connection so the
compositor withholds privileged globals [L-74][L-75].

### 1.5 The plumbing

XDG portals gained input capture, clipboard, notification v2 with sound and
category, USB, a host application registry, configurable global shortcuts, a
reduced-motion settings key and target selection for screenshots (1.19 to 1.21)
[L-78]; the Settings portal already carries colour scheme and an accent colour
negotiated across five desktops [L-79]. Flatpak's sandbox is undermined in
practice: roughly 42% of Flathub applications override it with home or host
filesystem access, two 2024 CVEs hit the portal bridge, and a portal grant
silently overrides static policy set from outside the sandbox [L-80][L-81].
PipeWire 1.6 is one graph for audio, video and capture [L-84]. systemd homed
gives portable encrypted homes; 259 made journald persistent by default and
added recovery keys and sysext mutability [L-85][L-86][L-87].

Atomic distributions buy A/B rollback from the boot menu and a clean split of
base, applications and data; SteamOS has run that model for millions since 2022
[L-90][L-93][L-88][L-89]. The costs come from people who lived with them:
layering "tended to break things more often than not", tooling needed about 20
hours of workarounds for one author, Docker and editor integration are rough,
and opinionated images make swapping a default tool hard [L-92][L-91][L-90]. The
summary from one user: "I want Linux to be my OS, not my hobby" [L-90].

### 1.6 Cross-cutting

Theming culture is total: Catppuccin has 300 to 400 official ports sharing
accent names across four flavours [L-97]; matugen generates palettes from the
wallpaper and templates dozens of application configurations [L-98]. Mutter is
client-side-decoration only, KWin defaults to server-side, COSMIC supports both;
the strongest argument against client decorations remains that a frozen window
cannot be closed from its own frame [L-99][L-100]. AT-SPI's weakness is
chattiness and no way to trust events under sandboxes; Newton pushes the
accessibility tree through the compositor so the assistive technology already
has it, and AccessKit landed in GTK 4.18 carrying that protocol [L-95][L-17].
Input methods are the most fragmented layer: two text-input versions, two
input-method versions, per-desktop behaviour, and the COSMIC launch failure
[L-102][L-58]. Printing is moving to sandboxed printer applications behind IPP
Everywhere [L-103].

### 1.7 Verdicts carried forward

- One toolkit and one style source from day one; the theme is a document that
  applications read, never a per-application skin [L-37][L-8].
- The Settings-portal contract (appearance, accent, reduced motion, fonts) as an
  appearance bus every application must honour, extended with contrast [L-79][L-78].
- Tiling as a per-workspace mode with stacks as a compositor primitive, and a
  scrollable strip as a first-class layout [L-53][L-59].
- Per-monitor workspaces by default; one organising hierarchy, not two [L-55][L-45].
- The greeter runs the real shell code path; first-run setup is separate from
  the installer [L-38][L-28].
- Push-model accessibility through the compositor from the first release [L-95].
- Session restore as a compositor primitive on day one [L-21].
- Privileged shell protocols (layer surfaces, clipboard history, foreign
  toplevels, capture) granted by capability to shell components, then a
  clipboard manager, screenshot tool and taskbar shipped in the box [L-75][L-70].
- One permission model regardless of packaging; static policy is absolute unless
  the user grants an exception, and every grant is logged [L-36][L-81].
- A compositor-owned background-application area with a strict API, not
  arbitrary tray pixmaps [L-14][L-29].
- Image-based system with A/B slots, automatic rollback, layered extensions and
  a safe mode, plus an inventoried mechanism for drivers [L-22][L-40][L-91].
- One input-method owner in the compositor [L-102].
- Anti-patterns: extensions as the customisation API; a second theming system;
  two overlapping workspace concepts; configuration-format churn; portal grants
  that override policy; removing a feature without an equal replacement;
  protocol-version matrices between compositor, toolkit and daemon; leaving
  session restore, clipboard, accessibility and colour management for later.

### Sources for section 1

[L-1] https://release.gnome.org/50/
[L-2] https://www.omgubuntu.co.uk/2026/03/gnome-50-released
[L-3] https://release.gnome.org/49/
[L-4] https://release.gnome.org/48/
[L-5] https://www.heise.de/en/news/GNOME-49-retires-X11-10437523.html
[L-8] https://planet-gnome-rs.livejournal.com/13813152.html
[L-9] https://www.phoronix.com/news/libadwaita-1.8-Released
[L-10] https://nyaa.place/blog/libadwaita-1-9/
[L-11] https://developer.gnome.org/hig/principles.html
[L-14] https://discourse.gnome.org/t/system-tray-icons-in-gtk4/22615
[L-15] https://linuxiac.com/how-to-enable-system-tray-icons-in-gnome/
[L-17] https://blogs.gnome.org/tbernard/2025/04/11/gnome-stf-2024/
[L-21] https://linuxiac.com/gnome-is-building-session-save-and-restore-but-it-wont-make-gnome-51/
[L-22] https://www.phoronix.com/news/GNOME-OS-Safe-Mode
[L-23] https://en.wikipedia.org/wiki/GNOME_Shell
[L-24] https://kde.org/announcements/megarelease/6/
[L-25] https://kde.org/announcements/plasma/6/6.3.0/
[L-26] https://kde.org/announcements/plasma/6/6.4.0/
[L-27] https://kde.org/announcements/plasma/6/6.5.0/
[L-28] https://kde.org/announcements/plasma/6/6.6.0/
[L-29] https://kde.org/announcements/plasma/6/6.7.0/
[L-30] https://linuxiac.com/kde-plasma-6-8-expands-union-theming-to-qtwidgets-apps/
[L-31] https://www.theregister.com/2025/11/28/kde_6_8_wayland_only/
[L-33] https://pointieststick.com/2025/12/28/highlights-from-2025/
[L-35] https://blogs.kde.org/2025/05/17/this-week-in-plasma-hdr-calibration-wizard/
[L-36] https://blogs.kde.org/2025/09/05/this-week-in-plasma-more-app-permission-configuration-pre-akademy-edition/
[L-37] https://lwn.net/Articles/1071703/
[L-38] https://planet.kde.org/david-edmundson-2025-03-26-a-roadmap-for-a-modern-plasma-login-manager/
[L-39] https://www.phoronix.com/news/Fedora-44-KDE-To-Plasma-Login
[L-40] https://pointieststick.com/2025/09/06/announcing-the-alpha-release-of-kde-linux/
[L-41] https://lwn.net/Articles/1037166/
[L-42] https://pointieststick.com/2026/03/31/this-month-in-kde-linux-march-2026/
[L-45] https://discuss.kde.org/t/make-one-system-virtual-desktop-or-activities/27747
[L-51] https://en.wikipedia.org/wiki/COSMIC_desktop
[L-53] https://system76.com/blog/post/cosmic-de-tiling-redesign-and-libcosmic-rebasing
[L-54] https://system76.com/cosmic/
[L-55] https://botmonster.com/self-hosting/cosmic-desktop-1-0-daily-driving-system76-rust-de/
[L-58] https://github.com/pop-os/cosmic-session/issues/185
[L-59] https://github.com/niri-wm/niri
[L-60] https://github.com/niri-wm/niri/releases
[L-61] https://lwn.net/Articles/1025866/
[L-62] https://hypr.land/news/update55/
[L-64] https://forum.hypr.land/t/each-new-0-5x-version-changes-everything/1622
[L-66] https://codeberg.org/river/river
[L-68] https://wayland.app/protocols/
[L-70] https://wayland.app/protocols/ext-data-control-v1
[L-71] https://wayland.app/protocols/wlr-layer-shell-unstable-v1
[L-73] https://www.phoronix.com/news/Wayland-CM-HDR-Merged
[L-74] https://wayland.app/protocols/ext-foreign-toplevel-list-v1
[L-75] https://wayland.app/protocols/security-context-v1
[L-78] https://github.com/flatpak/xdg-desktop-portal/blob/main/NEWS.md
[L-79] https://www.phoronix.com/news/XDG-Desktop-Portal-1.17.1
[L-80] https://www.linuxjournal.com/content/when-flatpaks-sandbox-cracks-real-life-security-issues-beyond-ideal
[L-81] https://github.com/flatpak/flatpak/issues/3977
[L-84] https://www.phoronix.com/news/PipeWire-1.6
[L-85] https://github.com/systemd/systemd/releases/tag/v259
[L-86] https://www.phoronix.com/news/systemd-258
[L-87] https://systemd.io/HOME_DIRECTORY/
[L-88] https://universal-blue.org/
[L-89] https://bootc.dev/bootc/
[L-90] https://encyclopediaofdaniel.com/blog/bazzite-aurora-life-with-an-immutable-os/
[L-91] https://tedium.co/2024/12/12/bazzite-atomic-desktop-linux-review/
[L-92] https://wstyler.ucsd.edu/posts/silverblue_fixes.html
[L-93] https://en.wikipedia.org/wiki/SteamOS
[L-95] https://lwn.net/Articles/971541/
[L-97] https://catppuccin.com/blog/celebrating-three-years-of-catppuccin/
[L-98] https://github.com/InioX/matugen
[L-99] https://en.wikipedia.org/wiki/Client-side_decoration
[L-100] https://alternativeto.net/news/2025/1/terminal-emulator-ghostty-1-1-0-brings-server-side-decorations-ime-enhancements-and-more/
[L-101] https://biggo.com/news/202508230115_Clyp_Wayland_Support_Issues
[L-102] https://fcitx-im.org/wiki/Using_Fcitx_5_on_Wayland
[L-103] https://github.com/LinuxFoundationGSoC/ProjectIdeas/wiki/GSoC-2026-OpenPrinting
[L-110] https://www.codethink.co.uk/articles/2024/GNOME-OS-systemd-sysupdate/
[L-111] https://alternativeto.net/news/2025/12/system76-launches-first-stable-versions-of-pop-_os-24-04-lts-and-cosmic-desktop
[L-112] https://www.omgubuntu.co.uk/2026/02/kde-plasma-6-6-released

---

## 2. Windows 11 and macOS

As of 12 September 2026, Windows 11 version 26H2 is in Release Preview as an
enablement package on the shared 24H2/25H2/26H2 branch [W-20][W-19]; macOS 27
"Golden Gate" ships 14 September 2026, Apple-silicon only [W-93]. Isaac favours
Windows conventions on average, so this section is the longest and its verdicts
lean that way where Windows is better and say where macOS wins.

### 2.1 Fluent 2 as applied to Windows

The system is token-driven and small: Mica (opaque, wallpaper-tinted, for the
base layer of active windows) and Acrylic (translucent, for transient
light-dismiss surfaces only) [W-1]; three corner radii, 8 px for windows,
flyouts and dialogs, 4 px for in-page controls, 0 px where edges meet and when a
window is snapped or maximised [W-3]; six elevation levels built from a key
shadow plus an ambient one, with Windows substituting a stroke for the key
shadow and doubling shadow opacity in dark theme [W-4]; Segoe UI Variable with
weight and automatic optical-size axes, a ramp of 12/16, 14/20, 14/20 semibold,
18/24, 20/28, 28/36, 40/52 and 68/92, and no bold or italic in the ramp
(semibold for emphasis; italics excluded on dyslexia grounds) [W-5]; Segoe
Fluent Icons as 1 epx monoline glyphs on a square em, composed as base plus
modifier [W-6][W-7]; an accent generated with contrast guarantees in both modes
[W-9]. Motion is numeric: direct entrance cubic-bezier(0,0,0,1) at 167, 250 or
333 ms; point-to-point (0.55,0.55,0,1); direct exit 167 ms with a fade; fades
83 ms linear [W-10]. Window chrome is specified: a 32 px title bar (48 px with a
search box), a 16 px icon inset 16 px, every non-interactive title-bar pixel
draggable, right-click for the system menu, double-click to maximise, caption
buttons with rest, hover, pressed, active and inactive states [W-12]. The
spacing grid is 4 px with tokens 0 to 56 [W-11].

Why it works: a tiny vocabulary lets first-party surfaces feel like one product
cheaply; the stroke-instead-of-shadow rule stays crisp at 100% scale; the 0 px
rule avoids gaps in tiled layouts. Why it fails: it covers only the new shell
surfaces, and Win32, UWP and WinUI coexist. Developers document "the constant
need to fall back to Win32 P/Invoke" and that Microsoft's own flagship
applications use "internal controls, private APIs, or bespoke frameworks" [W-14].
When first-party teams avoid the public toolkit, third parties learn the truth
from that avoidance.

### 2.2 Taskbar, Start, Snap, desktops

Windows 11 launched without ungrouped labelled taskbar buttons (restored in 23H2,
characterised as "a bug fix for a (critical) regression" [W-21]) and without a
movable taskbar (restored in 26H2, five years on [W-18]). StartAllBack's pitch
is literally the list of what was dropped: ungrouped running apps, labels,
position, a drag-and-drop target [W-22]. The October 2025 Start is one
scrollable surface with pinned, recommended and an auto-categorised All apps
view (a category needs three apps or merges into Other), a Phone Link side
panel, and the Recommended section can at last be switched off [W-16][W-17].
Snap layouts appear on the maximise button or Win+Z; Snap groups restore whole
arrangements from the taskbar and appear in Alt+Tab and Task View; Win+arrows
snaps halves and quarters [W-23]. Reviewers still call Snap "Windows 11's most
impactful productivity feature" [W-72]. A virtual desktop switch changes every
monitor at once; there are no per-monitor desktops (product observation).

The Widgets board led with a news feed; in May 2026 Microsoft moved the feed to
a separate tab, turned off hover-launch and badges, and described the change as
"a calmer OS with fewer upsells or ads" [W-26][W-27].

### 2.3 Settings, Explorer, Terminal, PowerToys, winget

Settings has been absorbing Control Panel "for more than a decade"; Device
Manager, adapter details, printer properties, BitLocker and disk management
remain outside it, and the design lead's stated reason is driver compatibility
[W-28][W-29]. Explorer's two-tier context menu exists because registry-injected
shell extensions caused slow right-clicks and crashes; in June 2026 the Windows
design lead conceded the result "is still a cluttered mess" [W-30][W-34].
Explorer is slow enough that Microsoft began preloading it at boot; tests showed
memory rising from about 35 MB to 67 MB with launch still slower than Windows 10
[W-33][W-122]. Dark mode reached legacy file dialogs in August 2025 and broke
again in December [W-35][W-36].

Windows Terminal edits one documented settings file through a UI and a command
palette; 1.24 and 1.25 added a full Actions page, an Extensions page and
synchronized output [W-37]. PowerToys is Microsoft's own catalogue of unmet
shell needs: a Command Palette whose fuzzy matcher cut a lookup from 633 ms to
113 ms, FancyZones, Peek on Space, Advanced Paste with local models, Workspaces,
Alt-drag to move windows, a per-monitor dock, a shortcut guide that knows the
active application, and Window Hopper [W-39][W-40][W-41]. Project Zenith
(September 2026) ships a developer profile of Windows: Terminal and VS Code
pinned, Explorer showing extensions, hidden files and full paths, the Command
Palette in Search, tips and account notifications off [W-46]. winget 1.28 has
about 12,850 packages, YAML manifests, loadout import and export, and
Developer Configurations that take a fresh machine to ready-to-code in one
command [W-42][W-43][W-15]; users still call its errors "cryptic" beside
Chocolatey, and Scoop's no-admin user-directory installs remain a
differentiator [W-117].

### 2.4 Identity, agents, Recall

The November 2025 update let packaged applications register as passkey
providers gated by Windows Hello [W-49][W-125]. The June 2026 agent security
model gives agents distinct identities and a composable sandbox with process,
session (separate desktop, clipboard and input) and micro-VM isolation, with
audit [W-52]. That architecture is the only part of the AI push users did not
revolt against. Recall was pulled in 2024 after researchers found screenshots in
an unencrypted database readable by any local process, relaunched opt-in and
Hello-gated in 2025, and by January 2026 was reported internally as having
"failed in its current form" [W-55][W-54][W-57]. In November 2025 a post calling
Windows "an agentic OS" drew about 1.5 million views with replies running against
it, and Microsoft then cut Copilot entry points from Notepad, Snipping Tool,
Photos and Widgets [W-86][W-87][W-64].

### 2.5 What Windows users complain about, with evidence

Advertisements and "recommendations" in Start, Explorer, Settings and
notifications, each needing its own switch [W-78][W-79]; forced Microsoft
accounts, with the last local-account bypasses removed in 2025 [W-80][W-81];
pre-provisioned applications that return after updates [W-82]; three UI
generations [W-28][W-35]; enablement packages plus server-side feature flags, so
identical builds behave differently [W-18]; full-screen Edge prompts stopped only
in the EEA under the DMA [W-83][W-84]; web results inside local search, with a
plain toggle arriving only in June 2026 [W-85]; OneDrive silently relocating
Desktop and Documents into the cloud during setup [W-60].

### 2.6 What Windows users praise

A single shell modifier (Win+D, E, L, V, Z, Shift+S, arrows, numbers) [W-72];
windows, not applications, as the unit of Alt+Tab, plus Snap groups [W-23];
backward compatibility; a taskbar that stays visible when a window is maximised
and click-through focus on background windows, both of which switchers
immediately miss on macOS [W-73]; the drives-and-Quick-Access Explorer model;
gaming and driver breadth; enterprise manageability [W-76][W-77].

### 2.7 macOS Sequoia, Tahoe and Golden Gate

Liquid Glass drew the strongest design criticism of the period: light mode
"bleached white", dark mode "a moonless night", window backgrounds moved to 100%
white and lost separation, controls "get bigger when you interact with them",
corner radii that change with content (add a toolbar and the corners change),
and a Reduce Transparency switch broken in two releases [W-89][W-90][W-91].
Golden Gate adds a glass slider, higher opacity, standardised radii and uniform
toolbars [W-92][W-93]. Tahoe's Spotlight is the best launcher on any desktop:
actions from applications and the system, quick keys generated from usage,
clipboard history, and browsing of applications, files, actions and the
clipboard; the reviews' consensus was "Spotlight shines, Liquid Glass
disappoints" [W-94][W-95]. Stage Manager is "fiddly" and "rigid" [W-97];
Sequoia's tiling was "Apple's third go at fixing a problem" and halves-only
[W-96]. Sequoia introduced periodic re-authorisation of screen recording;
administrators report loops even when permissions are correct [W-103][W-104].
Switchers find click-to-focus, zoom-not-maximise, hidden chrome in fullscreen,
no labelled taskbar and per-application Cmd-Tab worse [W-73].

### 2.8 Third-party Windows tools as a specification of unmet needs

StartAllBack, Start11 and ExplorerPatcher restore the taskbar and menus and
survive updates by replacing shell components rather than patching the binary
[W-22][W-108][W-109][W-126]; Files adds column view, dual pane, tags and custom
columns [W-110]; Flow Launcher is an Alt+Space launcher with plugins and
Everything integration [W-111]; Everything answers filename search instantly
from the NTFS journal [W-112]; AutoHotkey is the automation layer Windows lacks
[W-113]; ShareX treats capture as a pipeline: region, window, scrolling, GIF,
then annotate, OCR, upload [W-114]; WinDirStat shows where space went [W-115];
Sysinternals is the diagnostics depth Task Manager lacks [W-116]; Scoop is the
package manager that needs no elevation [W-117].

### 2.9 Verdicts carried forward

- The complete taskbar on day one: labels, ungrouping, any edge, drop targets,
  jump lists, visible when maximised [W-21][W-18].
- macOS menu discipline rendered Windows-style: every command in a menu model
  with its shortcut, menus in the window, and the same model feeding a palette
  and context menus, so there is one command registry [W-73].
- Tahoe Spotlight's actions, quick keys and clipboard as the launcher model,
  keyboard-first and local [W-94][W-111].
- Everything's search model: a filesystem journal, instant filename search, no
  web results unless asked, sources labelled [W-112][W-18].
- Snap layouts, groups and Win+arrows, plus native custom grids and per-monitor
  desktops; reject Stage Manager [W-23][W-97].
- Click-through focus and true maximise; reject zoom-to-fit and hidden chrome [W-73].
- The precise chrome specification and the 0 px rule when tiled [W-12][W-3].
- Opaque tint as the base material, translucency for transient surfaces only,
  fixed radii, a transparency slider and a hard reduce-transparency switch [W-1][W-91].
- One settings application, no legacy panel, every page addressable [W-29].
- One toolkit used by every first-party application, with no private controls [W-14].
- Tokens as OS API so third parties inherit them and contrast themes work by
  construction [W-3][W-10][W-120].
- Local-first identity: setup never requires a network or a cloud account; never
  auto-redirect user folders [W-80][W-60].
- One package manager with user-scope installs, machine scope for administrators,
  declarative machine configuration and legible errors; a developer profile as the
  default [W-117][W-15][W-46].
- Permissions with categories and persistence, a live indicator and an audit log,
  no periodic re-consent [W-103].
- Capture as a pipeline; clipboard history with pins; quiet desktop widgets or
  none [W-114][W-41][W-26].
- Agents, if any, as separate identities in isolated sessions with audit, and no
  brand buttons in core applications; no always-on capture [W-52][W-64][W-55].
- Anti-patterns: launching with less customisation than the predecessor; a
  two-tier context menu; web results in local search; feed-first widgets;
  offline-setup whack-a-mole; a decade of partial migration; fixing launch
  latency by preloading; translucency that breaks legibility; re-consent prompts;
  server-side feature flags; first-party apps on private controls; removing
  power-user escape hatches.

### Sources for section 2

[W-1] https://fluent2.microsoft.design/material
[W-3] https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/geometry
[W-4] https://fluent2.microsoft.design/elevation
[W-5] https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/typography
[W-6] https://learn.microsoft.com/en-us/windows/apps/design/iconography/
[W-7] https://learn.microsoft.com/en-us/windows/apps/design/iconography/segoe-fluent-icons-font
[W-9] https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/color
[W-10] https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/motion
[W-11] https://fluent2.microsoft.design/layout
[W-12] https://learn.microsoft.com/en-us/windows/apps/design/basics/titlebar-design
[W-14] https://windowsforum.com/threads/build-2026-microsoft-recommits-to-winui-and-windows-app-sdk-with-ai-tooling.422827/
[W-15] https://blogs.windows.com/windowsdeveloper/2026/06/02/build-2026-furthering-windows-as-the-trusted-platform-for-development/
[W-16] https://www.neowin.net/news/windows-11-25h2-and-24h2-get-the-new-start-menu-and-a-lot-of-new-features-in-kb5067036/
[W-17] https://kartikmehtablog.com/windows-11-25h2-new-start-menu/
[W-18] https://www.windowslatest.com/2026/08/30/i-tested-windows-11-26h2-so-you-dont-have-to-heres-everything-i-found/
[W-19] https://www.theregister.com/os-platforms/2026/08/28/microsoft-preps-this-years-windows-11-feature-tweaks-26h2-hits-release-preview/5293211
[W-20] https://blogs.windows.com/windows-insider/2026/08/27/releasing-windows-11-version-26h2-to-the-release-preview-channel/
[W-21] https://www.bleepingcomputer.com/news/microsoft/windows-11-finally-gets-a-never-combine-taskbar-buttons-mode/
[W-22] https://www.startallback.com/
[W-23] https://support.microsoft.com/en-us/windows/snap-your-windows-885a9b1e-a983-a3b1-16cd-c531795e6241
[W-26] https://www.windowslatest.com/2026/05/03/microsoft-is-finally-turning-off-the-msn-feed-and-ads-in-windows-11-widgets-by-default/
[W-27] https://www.windowscentral.com/microsoft/windows-11/microsoft-will-hide-windows-11s-annoying-msn-feed-by-default-as-it-moves-to-reduce-ads-and-noise-across-the-os
[W-28] https://www.ghacks.net/2025/12/02/windows-11-ten-years-after-it-started-microsoft-is-still-moving-control-panel-options-to-settings/
[W-29] https://www.windowslatest.com/2026/04/07/microsoft-explains-why-it-still-cant-fully-kill-control-panel-in-windows-11/
[W-30] https://www.windowslatest.com/2026/06/04/microsoft-admits-windows-11s-right-click-menu-is-a-mess-will-let-you-customize-it-after-years-of-complaints/
[W-33] https://www.techspot.com/news/110446-windows-11-updated-file-explorer-remains-painfully-slow.html
[W-34] https://www.windowslatest.com/2026/05/06/microsoft-confirms-windows-11s-file-explorer-is-sluggish-and-preloading-isnt-the-only-fix-coming/
[W-35] https://windowsforum.com/threads/windows-11-dark-mode-extends-to-legacy-dialogs-in-august-2025-preview.378028/
[W-36] https://www.windowslatest.com/2025/12/02/microsoft-confirms-windows-11-update-accidentally-breaks-dark-mode-in-file-explorer-causing-white-flashes/
[W-37] https://devblogs.microsoft.com/commandline/windows-terminal-preview-1-25-release/
[W-39] https://devblogs.microsoft.com/commandline/powertoys-0-95-is-here-new-light-switch-utility-faster-command-palette-and-peek-with-spacebar/
[W-40] https://github.com/microsoft/PowerToys/releases
[W-41] https://www.neowin.net/news/powertoys-096-is-out-with-new-features-to-command-palette-light-switch-and-other-modules/
[W-42] https://en.wikipedia.org/wiki/Windows_Package_Manager
[W-43] https://github.com/microsoft/winget-cli
[W-46] https://blogs.windows.com/windowsdeveloper/2026/09/04/announcing-project-zenith-the-ready-to-code-windows-experience/
[W-49] https://techcommunity.microsoft.com/blog/windows-itpro-blog/windows-11-expands-passkey-manager-support/4467572
[W-52] https://blogs.windows.com/windowsdeveloper/2026/06/02/windows-platform-security-for-ai-agents/
[W-54] https://www.computerworld.com/article/2140187/microsoft-makes-windows-recall-opt-in-after-privacy-security-backlash.html
[W-55] https://doublepulsar.com/microsoft-recall-on-copilot-pc-testing-the-security-and-privacy-implications-ddb296093b6c
[W-57] https://winbuzzer.com/2026/01/31/microsoft-scales-back-windows-11-ai-after-user-backlash-xcxwbn/
[W-60] https://it.slashdot.org/story/24/06/25/0222219/windows-11-is-now-automatically-enabling-onedrive-folder-backup-without-asking-permission
[W-64] https://www.windowslatest.com/2026/03/21/microsoft-responds-to-microslop-criticism-by-scaling-back-copilot-in-windows-11-starting-with-notepad-and-other-apps/
[W-72] https://keyshortcuts.net/blog/windows-11-shortcuts-guide
[W-73] https://www.howtogeek.com/my-one-unexpected-gripe-after-switching-from-windows-to-mac/
[W-76] https://www.pdq.com/blog/intune-vs-group-policy-whats-best-for-managing-windows-devices/
[W-77] https://thehgtech.com/guides/windows-vs-macos-endpoint-security-2026.html
[W-78] https://www.xda-developers.com/disable-windows-11-ads-by-changing-these-settings/
[W-79] https://www.windowscentral.com/software-apps/windows-11/how-to-disable-annoying-ads-on-windows-11
[W-80] https://pureinfotech.com/microsoft-blocks-windows-11-microsoft-account-bypass/
[W-81] https://www.bleepingcomputer.com/news/microsoft/microsoft-blocks-more-tricks-to-skip-microsoft-account-setup-in-windows-11/
[W-82] https://windowsforum.com/threads/windows-11-debloating-why-users-want-a-lighter-more-private-more-trusted-pc.415329/
[W-83] https://www.theregister.com/2025/06/03/microsoft_stops_eu_edge_pestering/
[W-84] https://www.techradar.com/computing/edge/fed-up-with-prompts-to-use-edge-windows-11-users-in-europe-wont-get-them-anymore-but-sadly-everyone-else-will
[W-85] https://www.windowslatest.com/2026/06/18/microsoft-announces-you-can-kill-bing-in-windows-11-search-and-boost-performance-after-years-of-lag/
[W-86] https://www.tomshardware.com/software/windows/top-microsoft-execs-boast-about-windows-evolving-into-an-agentic-os-provokes-furious-backlash
[W-87] https://4sysops.com/archives/microsoft-faces-massive-backlash-over-windows-11-agentic-os-plans-the-ai-naysayers-come-out-of-hiding/
[W-89] https://eshop.macsales.com/blog/97650-blurry-or-beautiful-the-tweaks-and-tenets-of-apples-controversial-liquid-glass-design-in-macos-tahoe/
[W-90] https://mjtsai.com/blog/2025/12/29/liquid-glass-disbelief/
[W-91] https://cloudship.co.uk/blog/macos-tahoe-liquid-glass/
[W-92] https://www.macrumors.com/2026/06/09/macos-golden-gate-liquid-glass/
[W-93] https://www.macworld.com/article/3139330/macos-27-mac-features-siri-apple-intelligence-release-date-compatibility.html
[W-94] https://9to5mac.com/2025/06/10/macos-26-spotlight-gets-actions-clipboard-manager-custom-shortcuts/
[W-95] https://9to5mac.com/2025/11/04/macos-tahoe-26-1-adds-new-tools-for-spotlights-clipboard-feature/
[W-96] https://appleinsider.com/articles/24/07/01/window-tiling-in-macos-sequoia-is-apples-third-go-at-fixing-a-problem
[W-97] https://taoofmac.com/space/blog/2023/05/20/1130
[W-103] https://www.macrumors.com/2024/08/15/macos-sequoia-screen-recording-app-permissions/
[W-104] https://www.screenify.studio/blog/2026-04-23-macos-screen-recording-permissions
[W-108] https://www.xda-developers.com/finally-set-up-explorerpatcher-windows-should-have-sooner/
[W-109] https://www.techradar.com/news/start11-continues-to-solve-windows-11s-taskbar-issues-before-sun-valley-2-arrives
[W-110] https://techpp.com/2026/04/30/files-best-file-explorer-alternative-windows/
[W-111] https://www.flowlauncher.com/
[W-112] https://www.voidtools.com/
[W-113] https://windowsforum.com/threads/top-open-source-windows-tools-for-power-users.378974/
[W-114] https://getsharex.com/
[W-115] https://en.wikipedia.org/wiki/WinDirStat
[W-116] https://blog.usro.net/2024/10/geek-tools-for-windows-essential-software-for-power-users-and-tinkerers/
[W-117] https://winget.tech/blog/winget-vs-chocolatey-vs-scoop
[W-120] https://learn.microsoft.com/en-us/windows/apps/design/accessibility/high-contrast-themes
[W-122] https://www.windowslatest.com/2025/11/28/tested-windows-11s-faster-file-explorer-preloaded-is-still-slower-than-windows-10-and-uses-additional-ram/
[W-125] https://www.bleepingcomputer.com/news/security/windows-11-now-supports-3rd-party-apps-for-native-passkey-management/
[W-126] https://github.com/valinet/ExplorerPatcher

---

## 3. Alternative, research and historical systems

This section is the one the brief cared about most: what to adapt, and what
sounded good and failed.

### 3.1 Plan 9 and 9front

Every resource is a file tree served over 9P, and every process assembles its
own private view with `mount` and `bind`, including union directories searched
in order; namespaces are cheap enough to be the mechanism for the window system,
networking, remote execution and even time travel by binding a historical
snapshot over a path [A-1]. rio's service to clients is identical to the service
it consumes: each window gets its own `/dev/cons`, `/dev/mouse` and `/dev/draw`,
so rio runs inside rio and any window can be driven by a program [A-2][A-3].
acme makes any text a command and exports every window as files so a 560-line
shell wrapper and a 1,200-line mailer extend it without reimplementing
interaction [A-4]. The plumber routes six-field messages by a user-editable
rule file, about 2,000 lines of C in total [A-5][A-6]. factotum holds every key
and negotiates authentication for any program, which eliminates setuid [A-7].
fossil and venti give daily snapshots at `/snapshot/yyyy/mmdd/hhmm` [A-9]. rc is
"not a macro processor": single-pass parsing, list-valued variables, one quoting
rule [A-10].

It "fell short of being a compelling enough improvement on Unix to displace its
ancestor" and Pike stopped daily-driving it for want of a C++ compiler, NFS and
a browser [A-11][A-12]. 9front keeps releasing (April and October 2025, January
and August 2026) [A-13][A-14].

### 3.2 Fuchsia

Zircon objects are reachable only through handles carrying rights; two handles
to one object may differ; a handle written into a channel leaves the sender;
process creation itself needs a job handle [A-15]. FIDL declares handle types
and rights in the interface and the runtime clamps handles to what the interface
declares [A-16]. The component framework makes the system a tree whose
manifests use, offer and expose capabilities, resolved by walking the tree with
the rule that a parent never consumes what a child exposes [A-17][A-18]. The
UI stack started as a 3D scene graph and was replaced by Flatland, a
2D-rectangles-only API, because the products were 2D and the legacy paths were
untested [A-19][A-20][A-21]. Starnix runs unmodified Linux binaries by trapping
syscalls into a userspace runtime [A-22]. The practical costs of pure
capabilities, from an academic review: no central filesystem makes coordination
awkward, calls can fail at runtime, and manifest management "encourages
over-permissioning" [A-23]. The team was cut in 2023, Chrome-on-Fuchsia ended in
2024, the visible work is a guest for pKVM, and Google's 2026 desktop direction is
Android [A-24][A-25][A-26].

### 3.3 Redox, SerenityOS, Haiku, Genode, Theseus, hobby systems

Redox names resources `scheme:path` served by userspace daemons; a process's
namespace is inherited, so sandboxing is a namespace restriction [A-27][A-28];
2025 to 2026 brought a priorities document, per-window fractional scaling, an
EEVDF scheduler and a grant for virtualized Redox [A-29][A-30][A-31].
SerenityOS is "a system by us, for us" with a 1990s retained widget toolkit
small enough for one person to understand; Ladybird forked out, accepted
third-party libraries Serenity refused, and is adopting Rust [A-32][A-33][A-34].
Haiku's BFS stores typed attributes, indexes them, and answers live queries that
are themselves saved documents; Tracker shows attributes as columns and is
spatial by default; everything is a message, so `hey` scripts any GUI by sending
what a click sends; replicants and translators exist, replicants "saw minimal
adoption"; R1 beta 6 shipped 26 August 2026 [A-36][A-37][A-39][A-40][A-42].
Genode is a tree where children are created from their parents' resources and
memory and CPU are traded down explicitly; Sculpt's admin view is the live
component graph, a click shows a component's trusted base and rules, and a
deploy file applies the moment it is written; 26.04 made every component
visible and replaceable [A-44][A-45][A-46][A-47]. Theseus pushes OS invariants
into the type system [A-49]. Essence, skiftOS and ToaruOS are one-person systems
whose authors call them long shots or artistic pursuits [A-51][A-53][A-54].
Hubris fixes every task at build time ("you can't write a fork-bomb without
fork"), uses synchronous IPC with leases, and parses untrusted pointers into
validated types; its debugger was co-designed with the OS so diagnostics stay
out of production images [A-55][A-56]. Barrelfish's retrospective: the core-count
prediction was wrong, adopting seL4-style memory capabilities was "ludicrously
complex" on hot-pluggable commodity hardware [A-57]. Oberon made any text on
screen a command; A2 tried a zooming UI [A-59][A-60]. Smalltalk's image isolates
the system from files, diffs and source control [A-61][A-63]. Arcan is a display
server that is a scriptable engine with a first-class text-UI API, and its
author's case against terminal emulation is in section 4 [A-65][A-66].

### 3.4 Capability-secure UI research

CapDesk and Polaris applied least authority to ordinary applications: an
installation endowment, then the user's ordinary acts of designation (picking a
file, double-clicking a document) are the authorization; the trusted dialog is
the powerbox [A-68]. The pattern is now mainstream without the name: the
Flatpak file-chooser portal and Android's photo picker hand a sandboxed
application exactly what the user chose, with no runtime permission [A-69][A-70].
Yee's ten principles of secure interaction design remain the checklist [A-71].
Access Control Gadgets embed a system-rendered control in the application and
treat the authentic click as the grant; Chrome's permission element is the same
idea on the web, and its measured effect is fewer errors and far better recovery
from a prior denial [A-72][A-73]. Qubes draws unforgeable coloured window
labels from outside the compartment and uses preloaded disposables [A-74][A-75].
The common thread: prompt fatigue and over-permissioning both come from asking
the user about authority instead of deriving it from what they are already
doing; Sculpt's alternative is to make the graph the primary view so trust is a
visible edit [A-23][A-73][A-45].

### 3.5 Data and document models

Attributes plus live queries work because the OS, not each application, owns
the metadata and the index [A-36][A-76]. WinFS "wasn't actually a filesystem"
but "SQL Server duct-taped to the OS", slowed setup and boot, gutted Explorer of
familiar navigation, and died with the Longhorn reset [A-77][A-78]; TMSU's lesson
is to keep tags as an extra view that never modifies files [A-79]. Newton soups
are the template for typed records with an index [A-80]. OpenDoc failed on
memory, hostile divisions, a cumbersome API and no economy for parts, and each
successor hit the same wall [A-81]. Local-first prototypes showed CRDTs work and
users feel ownership; Automerge 3 cut memory by up to 100x, and the malleable
software essay names the open problems: security of untrusted modifications, UI
composition, schema negotiation [A-82][A-83][A-84]. Time Machine, File History,
Nix generations and Plan 9's dump are one idea in four costumes: snapshots as a
navigable place [A-85][A-86][A-87][A-9]. Perkeep signs content-addressed claims
[A-90]. Hiding the filesystem broke the moment two applications touched one
document; Apple shipped Files in 2017 [A-92][A-93].

### 3.6 Shell and interaction research

Mercury OS's intents-as-spaces are "very much still Modal" by its author's own
critique [A-94]. Humane and Rabbit removed direct manipulation and every latency
became total failure [A-95][A-96]. The command palette is the one new shell that
won on the desktop: typed, fuzzy, context-aware, adjacent to the GUI rather than
replacing it [A-97][A-98]. Arc's 2025 letter reported a "novelty tax": 5.52% of
daily users touched multiple Spaces, and the company retreated to a conventional
layout [A-99]. visionOS shipments collapsed; spatial windows did not change what
windows are for [A-100][A-101]. Zoom elements are everywhere and "the grand
vision of a zoomable desktop has not materialized" [A-102][A-103]. Raskin's LEAP
(hold a key, type, jump) and modelessness survive as ideas; Archy shipped
without its zoom code [A-89][A-104]. Dynamicland is a place, not a product
[A-106]. Glamorous Toolkit is the most complete answer to "the shell as a
notebook of inspectable objects" [A-107]. PaperWM and niri are the scrollable
desktops that did not punish opening windows [A-108][A-109]. Structured shells:
nushell pipes typed values; YSH is "exterior-first" with real processes and
structure layered on JSON-like text [A-111][A-112][A-113].

### 3.7 Updates and trust

A/B slots with rollback protection (Android, ChromeOS), a sealed system volume
with snapshots (macOS), SteamOS's immutable root, NixOS and Guix generations
from the boot menu, and Debian at 93.9% reproducible packages; the user-facing
story of reproducibility is weak everywhere [A-114][A-115][A-116][A-117][A-118][A-87][A-119][A-121].

### 3.8 Verdicts carried forward

- Per-process namespaces built from capabilities as the sandbox primitive [A-1][A-28].
- The window server serves exactly what it consumes, so nesting, remoting,
  scripting and testing fall out [A-2][A-3].
- Content-based routing with a live, user-editable rule document [A-5].
- An agent that owns every key [A-7].
- Rights declared at the interface boundary and clamped there [A-15][A-16].
- Manifest-declared routing through an acyclic component tree, with a UI [A-17][A-45].
- The runtime graph as the admin view; parents pay for children [A-45][A-44].
- Designation is authorization; unspoofable system-rendered controls; unforgeable
  window labels [A-68][A-72][A-74].
- Typed attributes with an OS-owned index and live queries as documents; format
  translators as confined services; everything scriptable through the messages
  the UI itself sends [A-36][A-41][A-39].
- Snapshots as a place; content-addressed, signed history [A-9][A-85][A-90].
- A structured shell without macro processing, with a bytes escape hatch [A-111][A-10].
- Text as command; a command palette as the verb entry with a quasimode key [A-59][A-97][A-89].
- The scrollable strip [A-108].
- Static task graphs for the base system; a co-designed debugger [A-55][A-56].
- Moldable inspectors in a notebook shell [A-107].
- Immutable base, A/B, generations and safe mode in one panel; a hash-addressed
  store from day one [A-117][A-88][A-87].
- What failed and why: a relational filesystem as the OS's face; compound
  documents; hiding the filesystem; the image as the unit of software; the
  zoomable desktop; a 3D scene graph compositor; manifest-only capabilities with
  no UI; intent-first shells that stay modal; agents without direct
  manipulation; structured workspaces as a mass default; a different system with
  no ecosystem; multikernel as a scalability play.

### Sources for section 3

[A-1] https://9p.io/sys/doc/names.html
[A-2] https://doc.cat-v.org/plan_9/3rd_edition/rio/rio_slides.pdf
[A-3] https://man.cat-v.org/plan_9/4/rio
[A-4] https://9p.io/sys/doc/acme/acme.html
[A-5] https://9p.io/sys/doc/plumb.html
[A-6] https://9p.io/wiki/plan9/using_plumbing/index.html
[A-7] https://9p.io/sys/doc/auth.html
[A-9] http://man.cat-v.org/plan_9/4/fossil
[A-10] https://9p.io/sys/doc/rc.html
[A-11] http://www.catb.org/esr/writings/taoup/html/plan9.html
[A-12] https://interviews.slashdot.org/story/04/10/18/1153211/rob-pike-responds
[A-13] https://9front.org/releases/2026/08/02/0/
[A-14] https://www.osnews.com/story/143526/9front-release-released/
[A-15] https://fuchsia.dev/fuchsia-src/concepts/kernel/concepts
[A-16] https://fuchsia.dev/fuchsia-src/concepts/fidl/life-of-a-handle
[A-17] https://fuchsia.dev/fuchsia-src/concepts/components/v2/capabilities
[A-18] https://fuchsia.dev/reference/cml
[A-19] https://fuchsia.dev/fuchsia-src/contribute/governance/rfcs/0166_ui_stack
[A-20] https://fuchsia.dev/fuchsia-src/concepts/ui/scenic/flatland
[A-21] https://fuchsia.dev/fuchsia-src/contribute/governance/rfcs/0162_flatland
[A-22] https://fuchsia.dev/fuchsia-src/contribute/governance/rfcs/0082_starnix
[A-23] https://blog.mi.hdm-stuttgart.de/index.php/2023/07/30/fuchsia-rethinking-os-security-design-after-50-years/
[A-24] https://www.osnews.com/story/135778/googles-fuchsia-and-area-120-see-significant-cuts-in-layoffs/
[A-25] https://9to5google.com/guides/fuchsia/
[A-26] https://www.androidpolice.com/google-fuchsia-guide/
[A-27] https://lwn.net/Articles/979524/
[A-28] https://doc.redox-os.org/book/schemes.html
[A-29] https://www.redox-os.org/news/development-priorities-2025-09/
[A-30] https://www.redox-os.org/news/this-month-260630/
[A-31] https://www.phoronix.com/news/Redox-OS-June-2026
[A-32] https://serenityos.org/
[A-33] https://awesomekling.github.io/Introduction-to-SerenityOS-GUI-programming/
[A-34] https://en.wikipedia.org/wiki/Ladybird_(web_browser)
[A-36] https://www.haiku-os.org/docs/userguide/en/queries.html
[A-37] https://www.haiku-os.org/docs/userguide/en/tracker.html
[A-39] https://www.haiku-os.org/blog/humdinger/2017-11-05_scripting_the_gui_with_hey/
[A-40] https://www.haiku-os.org/documents/dev/replicants_more_application_than_an_application/
[A-41] https://hackmag.com/security/haiku
[A-42] https://www.haiku-os.org/news/2026-08-26_haiku_r1_beta6/
[A-44] https://genode.org/documentation/general-overview/index
[A-45] https://genode.org/documentation/articles/sculpt-26-04
[A-46] https://genode.org/documentation/articles/sculpt-25-10
[A-47] https://genode.org/download/sculpt
[A-49] https://www.usenix.org/conference/osdi20/presentation/boos
[A-51] https://gitlab.com/nakst/essence
[A-53] https://github.com/skift-org/skift
[A-54] https://toaruos.org/
[A-55] https://oxide.computer/blog/hubris-and-humility
[A-56] https://cliffle.com/blog/on-hubris-and-humility/
[A-57] https://www.sigops.org/2021/fish-in-a-barrel-an-insiders-retrospective-of-the-sosp09-multikernel-paper/
[A-59] https://en.wikipedia.org/wiki/Oberon_(operating_system)
[A-60] https://en.wikipedia.org/wiki/A2_(operating_system)
[A-61] https://en.wikipedia.org/wiki/Pharo
[A-63] https://news.ycombinator.com/item?id=8797522
[A-65] https://arcan-fe.com/about/
[A-66] https://arcan-fe.com/2024/12/25/arcan-0-7-the-all-tomato/
[A-68] https://dl.acm.org/doi/10.1145/1151030.1151033
[A-69] https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.FileChooser.html
[A-70] https://developer.android.com/training/data-storage/shared/photopicker
[A-71] https://www2.eecs.berkeley.edu/Pubs/TechRpts/2002/CSD-02-1184.pdf
[A-72] https://www.microsoft.com/en-us/research/publication/user-driven-access-control-rethinking-permission-granting-in-modern-operating-systems-2/
[A-73] https://developer.chrome.com/blog/rethinking-web-permissions
[A-74] https://doc.qubes-os.org/en/latest/introduction/intro.html
[A-75] https://doc.qubes-os.org/en/latest/developer/releases/4_3/release-notes.html
[A-76] http://www.nobius.org/dbg/practical-file-system-design.pdf
[A-77] https://getwired.com/2024/06/23/recalling-winfs/
[A-78] https://learn.microsoft.com/en-us/archive/blogs/jmazner/what-happened-to-winfs
[A-79] https://tmsu.org/
[A-80] https://en.wikipedia.org/wiki/NewtonScript
[A-81] https://instadeq.com/blog/posts/why-opendoc-failed-and-then-failed-3-more-times/
[A-82] https://www.inkandswitch.com/essay/local-first/
[A-83] https://www.inkandswitch.com/newsletter/dispatch-011/
[A-84] https://www.inkandswitch.com/essay/malleable-software/
[A-85] https://en.wikipedia.org/wiki/Time_Machine_(macOS)
[A-86] https://en.wikipedia.org/wiki/File_History
[A-87] https://nixos.org/guides/how-nix-works/
[A-88] https://wiki.nixos.org/wiki/Nixos-rebuild
[A-89] https://en.wikipedia.org/wiki/Archy_(software)
[A-90] https://en.wikipedia.org/wiki/Perkeep
[A-92] https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/FileSystemProgrammingGuide/FileSystemOverview/FileSystemOverview.html
[A-93] https://www.techradar.com/news/apple-files-app-has-been-released-and-removed-from-the-app-store
[A-94] https://www.mercuryos.com/
[A-95] https://techcrunch.com/2025/02/18/humanes-ai-pin-is-dead-as-hp-buys-startups-assets-for-116m
[A-96] https://en.wikipedia.org/wiki/Rabbit_r1
[A-97] https://eshop.macsales.com/blog/98115-spotlight-gets-supercharged-in-macos-tahoe-your-complete-guide-to-getting-more-out-of-the-default-mac-launcher/
[A-98] https://www.dottie.ai/blog/raycast-vs-alfred/
[A-99] https://browsercompany.substack.com/p/letter-to-arc-members-2025
[A-100] https://www.apple.com/newsroom/2025/06/visionos-26-introduces-powerful-new-spatial-experiences-for-apple-vision-pro/
[A-101] https://futureofvirtualreality.com/spatial-computing-in-2026/
[A-102] http://www.cs.umd.edu/~bederson/images/pubs_pdfs/2011_-_BIT_-_Promise_of_ZUIs.pdf
[A-103] http://www.cs.columbia.edu/graphics/courses/csw4170/resources/bedersonHollanUIST94.pdf
[A-104] https://en.wikipedia.org/wiki/The_Humane_Interface
[A-106] https://dynamicland.org/
[A-107] https://gtoolkit.com/
[A-108] https://lwn.net/Articles/1025866/
[A-109] https://github.com/niri-wm/niri
[A-111] https://www.nushell.sh/book/thinking_in_nu.html
[A-112] https://www.oilshell.org/blog/2023/06/ysh-design.html
[A-113] https://github.com/oils-for-unix/oils/wiki/Alternative-Shells
[A-114] https://source.android.com/docs/core/ota/ab
[A-115] https://source.android.com/docs/security/features/verifiedboot
[A-116] https://www.chromium.org/chromium-os/chromiumos-design-docs/verified-boot/
[A-117] https://support.apple.com/guide/security/signed-system-volume-security-secd698747c9/web
[A-118] https://en.wikipedia.org/wiki/SteamOS
[A-119] https://www.techradar.com/reviews/guix-system
[A-121] https://tests.reproducible-builds.org/debian/reproducible.html

---

## 4. Shells, terminals and editors

### 4.1 Shells

fish 4.0 (February 2025) replaced its C++ core with Rust; 4.1 added multi-line
autosuggestions, transient prompts and OSC 133 prompt marks with click-to-move;
4.2 made UTF-8 the default everywhere [T-8]. starship, zoxide and atuin are
assumed baseline: atuin stores every command with exit code, duration, cwd,
hostname and session in an encrypted database and replaces Ctrl-R with a
filterable view [T-13][T-15][T-16]. The Rust coreutils ship by default on Ubuntu
since 25.10 [T-17].

Structured pipelines are proven with caveats. nushell's tables and data
exploration are widely praised, and its users report configuration breakage
"multiple" times in six months, stream type inference that passes on the first
records and fails on the whole stream, no query planner, and the cost of
relearning every command [T-3]; in May 2026 the project opened an issue asking
whether to pursue 1.0 at all [T-1][T-2][T-5]. YSH argues for "exterior-first"
design: in-memory data structures serve wire formats, not the other way around
[T-6][T-7]. Murex attaches types to pipes while keeping shell syntax [T-10].
PowerShell's discoverability comes from a verb-noun grammar and a queryable verb
list, and its complaints are verbosity and the object-versus-CLI mismatch [T-9].
Interoperation with byte tools is where every structured shell's type errors
and quoting bugs concentrate [T-3][T-12].

### 4.2 Terminals

Every serious terminal is GPU-rendered, and the work is protocol extension on
top of VT: OSC 133 semantic prompts (iTerm2, WezTerm, kitty, VS Code, Ghostty,
foot; tmux exposes them as events) [T-30][T-31][T-38]; the kitty keyboard
protocol with five progressive-enhancement flags and a push/pop mode stack,
implemented by kitty, Alacritty, foot, Ghostty, iTerm2, Windows Terminal, Rio,
WezTerm, xterm.js and by Textual, Crossterm, Notcurses, Vim, Neovim, Emacs and
fish [T-24][T-33]; three image protocols with 15 of 44 terminals on Sixel and 9
on kitty's, which places images by id and z-index and survives multiplexers
through a Unicode placeholder [T-36][T-25][T-47]; kitty's text-sizing protocol
for headings in text UIs [T-26]; OSC 5522 clipboard with MIME types and user
consent for reads [T-27]. Ghostty 1.3.0 (March 2026) shipped click-to-move,
command-finish notifications, a multi-format clipboard and Unicode 17 grapheme
clustering [T-22]. Zellij runs plugins on WebAssembly with granular permissions
and added remote sessions and kitty graphics passthrough in 2026 [T-39][T-40].

Warp made the block "the fundamental unit" grouping a command and its output,
gives each block its own grid so cursor movement cannot corrupt an earlier one,
owns the input line in a CRDT-backed editor, built its own GPU toolkit, and
open-sourced the client in April 2026 [T-19][T-20][T-18][T-21]. Arcan's
"Sunsetting cursed terminal emulation" argues that terminals "emulate a machine
that has never existed", that in-band escape sequences are instruction
smuggling, and that emulators are stacked four deep; its replacement is a
shared-memory IPC with an explicit text-UI API [T-41][T-42]. libghostty's author
takes the other side: implement the existing zoo correctly first, because
hundreds of applications embed buggy emulators [T-23]. Every successful TUI
framework (Ratatui, Textual, Bubble Tea, Ink) converged on the same declarative,
reactive models as GUIs [T-43][T-44][T-45][T-46].

### 4.3 Editors

VS Code remains the default for about three quarters of respondents and
standardised the command palette, layered settings generated from schemas, LSP,
DAP and the extension model [T-54][T-55][T-56][T-57][T-58]. Helix is
selection-first with tree-sitter and LSP built in and no plugin system yet; its
maintainers reject Vim keybindings and public roadmaps and say it is "not meant
to be everything for everyone" [T-48][T-49]. Zed's GPUI is a hybrid
immediate/retained GPU toolkit; its Agent Client Protocol is JSON-RPC over stdio
positioned as "LSP for agents", co-developed with Google and adopted by
JetBrains [T-52][T-50][T-51]. Neovim is adding a built-in package manager, LSP
defaults and multiple cursors [T-53].

### 4.4 Verdicts carried forward

- Define the wire format first: a canonical, versioned, streaming structured
  encoding that every first-party tool speaks, with lossless fallback to bytes
  at the boundary [T-7][T-10].
- A verb-noun grammar with a queryable registry and machine-readable signatures,
  so completion, help, agents and the GUI derive from one schema [T-9][T-58].
- fish-grade defaults; history as a structured system service; a stability
  policy for scripts and configuration from day one [T-8][T-13][T-1][T-3].
- Decide the type-inference contract at boundaries so a stream cannot pass on
  its first rows and fail later [T-3].
- The native terminal is a session protocol of records (prompt, command, output,
  status, duration, cwd) with per-block grids; the byte grid is one renderer;
  VT100 is a compatibility component [T-20][T-30][T-41][T-23].
- Keyboard input defined once with press, repeat, release and alternates [T-24].
- Images, text sizes, hyperlinks and MIME clipboard native, with consent for
  clipboard reads [T-25][T-26][T-27][T-29].
- Multiplexing and remoting in the session layer [T-32][T-39].
- The TUI toolkit shares the GUI toolkit's declarative core [T-43][T-46].
- The first-party editor is a Helix, not a VS Code: selection-first, tree-sitter,
  LSP, DAP and ACP native, a deferred plugin surface, a CRDT buffer, and the OS
  text stack rather than a private one [T-48][T-50][T-56][T-57][T-20].

### Sources for section 4

[T-1] https://github.com/nushell/nushell/issues/18297
[T-2] https://www.nushell.sh/blog/
[T-3] https://lars.yencken.org/in-praise-of-nushell/
[T-5] https://github.com/nushell/nushell/discussions/14958
[T-6] https://github.com/oils-for-unix/oils
[T-7] https://www.oilshell.org/blog/2023/06/ysh-design.html
[T-8] https://fishshell.com/docs/current/relnotes.html
[T-9] https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.utility/get-verb?view=powershell-7.5
[T-10] https://murex.rocks/
[T-12] https://gist.github.com/pmarreck/b7bd1c270cb77005205bf91f80c4e130
[T-13] https://docs.atuin.sh/
[T-15] https://www.commandinline.com/starship-prompt-setup-guide/
[T-16] https://linuxconfig.org/modern-cli-tools-time-to-forget-ls-and-cat
[T-17] https://github.com/uutils/coreutils
[T-18] https://github.com/warpdotdev/warp
[T-19] https://docs.warp.dev/terminal/blocks
[T-20] https://www.warp.dev/blog/how-warp-works
[T-21] https://knightli.com/en/2026/05/07/warpdotdev-warp-open-source-agentic-terminal/
[T-22] https://ghostty.org/docs/install/release-notes/1-3-0
[T-23] https://mitchellh.com/writing/libghostty-is-coming
[T-24] https://sw.kovidgoyal.net/kitty/keyboard-protocol/
[T-25] https://sw.kovidgoyal.net/kitty/graphics-protocol/
[T-26] https://sw.kovidgoyal.net/kitty/text-sizing-protocol/
[T-27] https://sw.kovidgoyal.net/kitty/clipboard/
[T-29] https://gist.github.com/egmontkob/eb114294efbcd5adb1944c9f3cb5feda
[T-30] https://gist.github.com/tep/e3f3d384de40dbda932577c7da576ec3
[T-31] https://contour-terminal.org/vt-extensions/osc-133-shell-integration/
[T-32] https://wezterm.org/multiplexing.html
[T-33] https://github.com/microsoft/terminal/releases
[T-36] https://terminaltrove.com/compare/terminals/
[T-38] https://raw.githubusercontent.com/tmux/tmux/master/CHANGES
[T-39] https://zellij.dev/news/
[T-40] https://zellij.dev/news/stacked-resize-pinned-panes/
[T-41] https://arcan-fe.com/2025/01/27/sunsetting-cursed-terminal-emulation/
[T-42] https://github.com/letoram/arcan/wiki/TUI
[T-43] https://ratatui.rs/
[T-44] https://textual.textualize.io/
[T-45] https://github.com/charmbracelet/bubbletea
[T-46] https://github.com/vadimdemedes/ink
[T-47] https://akmatori.com/blog/terminal-graphics-protocols
[T-48] https://github.com/helix-editor/helix/discussions/14457
[T-49] https://github.com/helix-editor/helix/pull/8675
[T-50] https://zed.dev/blog/bring-your-own-agent-to-zed
[T-51] https://agentclientprotocol.com/
[T-52] https://github.com/zed-industries/zed/tree/main/crates/gpui
[T-53] https://raw.githubusercontent.com/neovim/neovim/master/runtime/doc/news.txt
[T-54] https://survey.stackoverflow.co/2025/technology
[T-55] https://visualstudiomagazine.com/articles/2025/08/01/stack-overflow-dev-survey-visual-studio-vs-code-hold-of-ai-ides-to-remain-on-top.aspx
[T-56] https://microsoft.github.io/language-server-protocol/
[T-57] https://microsoft.github.io/debug-adapter-protocol/
[T-58] https://code.visualstudio.com/docs/configure/settings

---

## 5. Toolkits, rendering, text and tokens

### 5.1 Toolkit architecture

The field converged on declarative view descriptions diffed against a retained
widget tree (React, SwiftUI, Compose, Xilem, Dioxus), rendered by GPU 2D engines
that precompile shaders or use compute and sparse-strip rasterisation. iced's
Elm architecture lets state mutate only in one place and so offers time-travel
debugging; it powers COSMIC [T-68][T-69]. egui's own README lists immediate
mode's costs: layout needs sizes before contents are known, full relayout every
frame, accessibility needing stable ids [T-67]. Xilem keeps a transient view
tree, a persistent widget tree and a view-state tree, identifies by id path, and
gives event handlers direct mutable access to application state with no
observer pattern; Masonry can render without a GPU; Vello's sparse-strip CPU
path benchmarks second only to Blend2D and ahead of Skia and Cairo; Parley
gained system-font enumeration and AccessKit integration [T-60][T-62][T-63][T-64].
Compose's rules generalise: view functions must be fast, idempotent and free of
side effects; recomposition skips unchanged inputs, may be cancelled, may run
every frame and in any order; state flows down, events bubble up [T-75]. Slint
compiles a DSL to native code; Dioxus 0.7 has a native renderer built from Stylo
CSS, Taffy flexbox and Vello with hot-patching of Rust [T-70][T-72]. Impeller
replaced Skia in Flutter to kill shader-compilation jank by precompiling every
shader and pipeline offline [T-74]. AccessKit is a Chromium-derived tree schema:
the toolkit pushes a full tree then incremental updates and adapters retain it
per platform; it works with immediate mode if elements have stable ids [T-76][T-77].
The minimal input-method contract is enabled, preedit with a cursor range,
commit, disabled, plus a cursor rectangle for candidate placement [T-78][T-164].
Material 3 and Fluent 2 both specify motion as duration and easing tokens with a
mandatory no-motion setting [T-79][T-81].

### 5.2 Typography and text

Inter is OFL with weights 100 to 900 and an optical-size axis [T-82]; Segoe UI
Variable exposes weight and automatic optical size [T-83]; Roboto Flex exposes
13 axes [T-84]; IBM Plex covers sans, serif, mono and nine scripts [T-85];
Atkinson Hyperlegible Next (2025) is OFL and designed for low vision [T-87][T-88].
Monospace: JetBrains Mono, Cascadia Code (variable, Windows Terminal's default),
Iosevka's 19 stylistic sets, and Monaspace's texture healing, which evens
density inside fixed advances with contextual alternates and needs no software
support beyond `calt` [T-96][T-97][T-98][T-95]. HarfRust is a safe-Rust HarfBuzz
port within 25% of HarfBuzz's speed; Parley composes it with Fontique, Skrifa
and ICU4X and validates against Web Platform Tests; cosmic-text pairs it with
swash [T-89][T-65][T-66]. The hard problems are circular: styling needs fonts,
font choice needs shaping, layout needs shaped widths; plus overlapping Arabic
glyphs under transparency, ligatures across style boundaries, colour emoji and
discontinuous bidi selections [T-90]. FreeType's guidance: blend in linear
light, gamma-correct at output, hint lightly [T-91]. Bitmap atlases, SDF and
MSDF each have a magnification ceiling; Slug's curve-evaluation patent was
terminated on 17 March 2026 with MIT reference shaders; Vello unifies text with
all other vector content through a glyph cache [T-92][T-93][T-94][T-62]. The
OFL permits bundling and selling as part of a bundle, requires shipping the
licence, and requires renaming a modified face with a reserved font name [T-99].

### 5.3 Theming and tokens

The Design Tokens Community Group's first stable format (28 October 2025) is
JSON with typed values, groups, references, colour spaces including OKLCH, and
composite types; modes are handled outside the core format [T-107][T-108].
Fluent 2 and libadwaita both use two tiers (global and semantic); libadwaita
derives a standalone accent from the accent background through an OKLab
lightness clamp so foreground use stays contrast-safe [T-102][T-103]. OKLCH
keeps lightness consistent across hues; Material's HCT drives palettes from a
seed with tone differences that map to contrast ratios [T-109][T-100][T-101].
Catppuccin has four flavours of 26 colours across hundreds of ports; base16 and
base24 give 16 or 24 slots with fixed meanings across 230 schemes and 70
templates [T-104][T-105][T-106]. CSS Color HDR anchors SDR white at 203 cd/m2 and
keeps UI at reference white while media may exceed it [T-110]. Windows contrast
themes are a separate palette at 7:1 with eight system colour roles, four
built-in themes users can edit, and the rule never to hard-code a colour [T-111].
Reduced motion maps to a setting on every platform; the guidance is reduce, do
not remove [T-112]. Icon sets and licences: Material Symbols (Apache 2.0, 2,500+
glyphs, fill, weight, grade and optical-size axes), Phosphor (MIT), Lucide
(ISC), Tabler (MIT, 6,184 icons on a 24 px grid), Fluent UI System Icons (MIT),
Adwaita (CC-BY-SA, share-alike) [T-113][T-114][T-115][T-116][T-117][T-118].

### 5.4 Verdicts carried forward

- A Xilem-shaped core: declarative view functions diffed into a retained widget
  tree, id-path identity, direct mutable state access in handlers, purity rules
  enforced by the language [T-60][T-75].
- The accessibility tree as a by-product of the widget tree, and the same tree
  the automation and test harness reads [T-76].
- One renderer abstraction with CPU and GPU backends, all shaders precompiled,
  text through the same rasteriser with a glyph cache [T-62][T-63][T-74].
- Flexbox and grid with logical axes; no cascade [T-72][T-162].
- Motion as tokens plus a system reduced-motion switch consulted by the toolkit [T-79][T-81][T-112].
- Input method as a first-class text-input protocol with preedit ranges and
  compositor-provided anchors [T-78][T-164].
- OFL faces only, automatic optical size, a type ramp in effective pixels with
  minimum sizes and no italic or bold in chrome; texture-healing alternates in
  the terminal font pipeline [T-83][T-99][T-95].
- Blend in linear light, hint lightly, grayscale antialiasing at high DPI [T-90][T-91].
- Themes as token documents in OKLCH; three orthogonal preferences (appearance,
  contrast, motion); the terminal palette a base24 slice of the same tokens;
  one variable icon font plus a permissive outline set [T-108][T-109][T-111][T-106][T-113][T-116].

### Sources for section 5

[T-60] https://raphlinus.github.io/rust/gui/2022/05/07/ui-architecture.html
[T-62] https://linebender.org/blog/tmil-25/
[T-63] https://github.com/linebender/vello
[T-64] https://linebender.org/blog/tmil-19/
[T-65] https://github.com/linebender/parley
[T-66] https://github.com/pop-os/cosmic-text
[T-67] https://github.com/emilk/egui
[T-68] https://github.com/iced-rs/iced
[T-69] https://byteiota.com/iced-0-14-rust-gui-gets-reactive-rendering-time-travel/
[T-70] https://slint.dev/blog/slint-1.13-released
[T-72] https://dioxuslabs.com/blog/release-070
[T-74] https://docs.flutter.dev/perf/impeller
[T-75] https://developer.android.com/develop/ui/compose/mental-model
[T-76] https://accesskit.dev/how-it-works/
[T-77] https://2026.rustweek.org/talks/matt/
[T-78] https://docs.rs/winit/latest/winit/event/enum.Ime.html
[T-79] https://m3.material.io/styles/motion/overview
[T-81] https://fluent2.microsoft.design/motion
[T-82] https://rsms.me/inter/
[T-83] https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/typography
[T-84] https://github.com/googlefonts/roboto-flex
[T-85] https://github.com/IBM/plex
[T-87] https://www.brailleinstitute.org/freefont/
[T-88] https://github.com/googlefonts/atkinson-hyperlegible-next
[T-89] https://github.com/harfbuzz/harfrust
[T-90] https://faultlore.com/blah/text-hates-you/
[T-91] https://freetype.org/freetype2/docs/hinting/text-rendering-general.html
[T-92] https://www.redblobgames.com/articles/sdf-fonts/
[T-93] https://mathewsachin.github.io/blog/2026/03/18/decade-of-slug.html
[T-94] https://hackaday.com/2026/03/20/slug-algorithm-for-on-gpu-rendering-of-fonts-with-bezier-curves-now-in-public-domain/
[T-95] https://github.com/githubnext/monaspace/blob/main/docs/Texture%20Healing.md
[T-96] https://www.jetbrains.com/lp/mono/
[T-97] https://github.com/microsoft/cascadia-code
[T-98] https://typeof.net/Iosevka/
[T-99] https://openfontlicense.org/ofl-faq/
[T-100] https://github.com/material-foundation/material-color-utilities
[T-101] https://www.androidauthority.com/material-3-expressive-color-3560841/
[T-102] https://fluent2.microsoft.design/design-tokens
[T-103] https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/css-variables.html
[T-104] https://github.com/catppuccin/catppuccin
[T-105] https://github.com/tinted-theming/home
[T-106] https://github.com/tinted-theming/base24/blob/main/README.md
[T-107] https://www.w3.org/community/design-tokens/2025/10/28/design-tokens-specification-reaches-first-stable-version/
[T-108] https://www.designtokens.org/tr/drafts/format/
[T-109] https://evilmartians.com/chronicles/oklch-in-css-why-quit-rgb-hsl
[T-110] https://www.w3.org/TR/css-color-hdr-1/
[T-111] https://learn.microsoft.com/en-us/windows/apps/design/accessibility/high-contrast-themes
[T-112] https://developer.mozilla.org/en-US/docs/Web/CSS/@media/prefers-reduced-motion
[T-113] https://developers.google.com/fonts/docs/material_symbols
[T-114] https://github.com/phosphor-icons/core
[T-115] https://lucide.dev/license
[T-116] https://github.com/tabler/tabler-icons
[T-117] https://github.com/microsoft/fluentui-system-icons
[T-118] https://gitlab.gnome.org/GNOME/adwaita-icon-theme
[T-162] https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_logical_properties_and_values
[T-164] https://fcitx-im.org/wiki/Using_Fcitx_5_on_Wayland

---

## 6. Packaging, services, configuration and logging

Nix stores every package under a hash of its inputs, gives each user a profile,
makes upgrades atomic and rollbacks instant; Guix Home applies the same to the
user environment; Fuchsia packages are content-addressed blob sets
[T-119][T-121][T-147]. bootc boots OCI images as the OS with transactional
updates [T-131][T-132]. Flatpak's runtimes, sandbox and portals are the model of
user-mediated access from a sandbox [T-122][T-123]; AppImage rejects sharing and
sandboxing; snapd confines with plugs and slots [T-124][T-125]. winget is
manifest-driven with declarative configuration; Scoop installs per user with no
elevation and shims instead of PATH pollution [T-126][T-127]. Distrobox is the
escape hatch on immutable systems and explicitly not a sandbox; dev containers
and mise standardise tool environments [T-128][T-129][T-130].

Configuration languages: TOML is minimal and nests poorly [T-133]; KDL 2.0 is
node-based, comment-friendly and version-tagged [T-134][T-135]; Nickel adds
gradual types and contracts [T-136]; Dhall is total and typed [T-137]; Pkl's
differentiator is schema validation [T-138]; CUE's unification forbids
overrides so a concrete value is final and order is irrelevant [T-139].

Service managers: systemd's exec hardening catalogue is an ambient-authority
reduction list [T-140]; launchd launches on demand from sockets declared in the
plist, not in code [T-141]; s6-rc compiles the service database offline with
atomic transitions to any consistent set [T-142]; dinit offers typed dependency
edges and user services [T-143][T-144]. Fuchsia components declare use, offer
and expose in manifests routed down a topology tree [T-145][T-146]; Sculpt
routes each requested service to a provider, including a black-hole
pseudo-service for untrusted components, from verified depots with on-target
rollback [T-148].

Logging: journald separates untrusted user fields from kernel-asserted trusted
fields, which is what makes filtering trustworthy [T-150]; OpenTelemetry
correlates by trace and span [T-151]; `tracing` spans restore causality in
asynchronous systems [T-152]; Fuchsia Inspect adds live hierarchical state per
component as a third signal [T-153].

### Verdicts carried forward

- Packages are immutable, content-addressed component sets with signed
  provenance; installation is profile mutation with generations and rollback [T-119][T-147].
- Every program is a component with a manifest naming what it uses and exposes;
  with no ambient authority, systemd's hardening list is the default [T-140][T-145].
- Services have explicit readiness and typed dependency edges, compiled to a
  verified graph before boot, launched on demand by capability request [T-141][T-142][T-143].
- Portals are the only path to user resources, for first-party applications too [T-123].
- One total configuration language: a node-shaped surface with typed schemas and
  no silent overrides [T-134][T-136][T-137][T-139].
- Structured logs with trusted fields injected by the kernel, span ids by the
  runtime, and per-component live state [T-150][T-152][T-153].
- A clearly labelled un-sandboxed compatibility container for foreign software [T-128].

### Sources for section 6

[T-119] https://nixos.org/guides/how-nix-works/
[T-121] https://guix.gnu.org/manual/en/html_node/Home-Configuration.html
[T-122] https://docs.flatpak.org/en/latest/introduction.html
[T-123] https://flatpak.github.io/xdg-desktop-portal/docs/
[T-124] https://docs.appimage.org/introduction/concepts.html
[T-125] https://github.com/canonical/snapd
[T-126] https://learn.microsoft.com/en-us/windows/package-manager/winget/
[T-127] https://github.com/ScoopInstaller/Scoop
[T-128] https://distrobox.it/
[T-129] https://containers.dev/
[T-130] https://mise.jdx.dev/
[T-131] https://bootc.dev/bootc/
[T-132] https://fedoraproject.org/wiki/Initiatives/Image_Mode,_Phase_2_(2026)
[T-133] https://toml.io/en/
[T-134] https://kdl.dev/
[T-135] https://github.com/kdl-org/kdl/releases
[T-136] https://nickel-lang.org/
[T-137] https://github.com/dhall-lang/dhall-lang
[T-138] https://pkl-lang.org/main/current/introduction/comparison.html
[T-139] https://cuelang.org/docs/concept/the-logic-of-cue/
[T-140] https://man7.org/linux/man-pages/man5/systemd.exec.5.html
[T-141] https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html
[T-142] https://skarnet.org/software/s6-rc/overview.html
[T-143] https://github.com/davmac314/dinit
[T-144] https://chimera-linux.org/docs/configuration/services
[T-145] https://fuchsia.dev/fuchsia-src/concepts/components/v2/introduction
[T-146] https://fuchsia.dev/fuchsia-src/concepts/components/v2/capabilities
[T-147] https://fuchsia.dev/fuchsia-src/concepts/packages/package
[T-148] https://genode.org/documentation/articles/sculpt-25-10
[T-150] https://man7.org/linux/man-pages/man7/systemd.journal-fields.7.html
[T-151] https://opentelemetry.io/docs/specs/otel/logs/
[T-152] https://docs.rs/tracing/latest/tracing/
[T-153] https://fuchsia.dev/fuchsia-src/development/diagnostics/inspect

---

## 7. Accessibility and internationalisation

UI Automation exposes a provider tree with control patterns serving screen
readers and test automation alike, and NVDA is the reference consumer [T-154][T-155].
GNOME's Newton flips AT-SPI's pull model: applications push accessibility state
to the compositor, the state is always synchronised with what is visible, and it
works inside sandboxes [T-156]. WCAG 2.2 adds focus-not-obscured, focus
appearance, dragging alternatives, minimum target size, consistent help and
accessible authentication [T-157]. Windows Live Captions transcribes any audio
on device in more than 20 languages [T-158]. CLDR 48 supplies locale data to
every major OS and ICU4X 2.0 delivers it in memory-safe Rust at 50 to 90% less
footprint [T-159][T-160]. Fluent's asymmetric localisation lets a simple source
string map to gendered, cased or multi-plural translations without source
changes [T-161]. Logical layout properties make right-to-left and vertical
scripts fall out of the layout system [T-162]. Windows TSF replaced IMM32 with
text stores and composition sessions; on Wayland the missing global coordinate
system for candidate popups is the classic pain [T-163][T-164].

### Verdicts carried forward

- The accessibility tree is produced for every widget, pushed to the compositor,
  and consumed by the screen reader, the automation harness and agent APIs [T-76][T-156].
- Keyboard-first: every action is a named command with a binding; focus is never
  obscured; focus appearance and target sizes follow WCAG 2.2 in the toolkit [T-157].
- A first-party screen reader, magnifier and on-device captions as OS components [T-155][T-158].
- ICU4X data and Fluent files as the only localisation path; CLDR formatters
  available to the shell [T-159][T-160][T-161].
- All layout in logical axes [T-162].
- An input-method protocol with per-surface state, preedit ranges and
  compositor-provided anchors [T-78][T-164].

### Sources for section 7

[T-154] https://learn.microsoft.com/en-us/windows/win32/winauto/entry-uiauto-win32
[T-155] https://www.nvaccess.org/about-nvda/
[T-156] https://blogs.gnome.org/a11y/2024/06/18/update-on-newton-the-wayland-native-accessibility-project/
[T-157] https://www.w3.org/WAI/standards-guidelines/wcag/new-in-22/
[T-158] https://support.microsoft.com/en-us/windows/use-live-captions-to-better-understand-audio-b52da59c-14b8-4031-aeeb-f6a47e6055df
[T-159] https://cldr.unicode.org/
[T-160] https://blog.unicode.org/2025/05/icu4x-20-released.html
[T-161] https://projectfluent.org/
[T-163] https://learn.microsoft.com/en-us/windows/win32/tsf/text-services-framework

---

## 8. What failed, and why

Collected from every angle, because a design that only reads successes repeats
the failures.

| Idea | Where | Why it failed | Lesson for this OS |
|---|---|---|---|
| A relational filesystem as the OS's face | WinFS | Slowed setup, boot and Explorer; broke familiar navigation; died with the reset [A-77][A-78] | Index and query beside the filesystem, never instead of it |
| Compound documents | OpenDoc, Bonobo, ActiveX, replicants | Memory, hostile divisions, awful APIs, no economy for parts [A-81][A-40] | Compose tools over shared data, not parts inside documents |
| Hiding the filesystem | iOS 2007 to 2017 | One-app-owns-a-document broke for two apps on one file [A-92][A-93] | Places and capabilities, never invisibility |
| The image as the unit of software | Smalltalk | Opaque to files, diffs and source control [A-63] | Documents and artifacts stay files |
| The zoomable desktop | Pad++, A2, Archy | Elements won, the metaphor disoriented [A-102] | Zoom as a control, not a desktop |
| A 3D scene graph compositor | Fuchsia Gfx | Products were 2D; legacy paths untested [A-19] | 2D rectangles only |
| Manifest-only capabilities with no UI | Fuchsia | Encourages over-permissioning; the user is never in the loop [A-23] | The graph is the UI; designation grants |
| Intent-first or spatial shells that stay modal | Mercury OS, visionOS | Intentions are still modes; shipments collapsed [A-94][A-101] | Modeless, with a quasimode palette |
| Agents without direct manipulation | Humane, Rabbit | Latency and misrecognition become total failure [A-95][A-96] | Agents are programs beside the direct UI |
| Structured workspaces as a mass default | Arc Spaces | 5.52% adoption, a novelty tax [A-99] | Ship experiments off by default with kill criteria |
| A different system with no ecosystem | Plan 9 | Not different enough to justify losing the browser and compiler [A-11][A-12] | Conform at the boundary; the annex exists for ports |
| Extensions as the customisation API | GNOME Shell | Every feature breaks each release [L-23][L-14] | Stable typed shell APIs |
| A second theming system | KDE before Union | Years to unify three generations [L-37] | One style source |
| Launching with less than the predecessor | Windows 11 taskbar | Five years of restoration [W-21][W-18] | The complete rail on day one |
| Translucency first, radii by content | macOS Tahoe | Legibility loss, broken accessibility switches [W-90][W-91] | Opaque base, fixed tokens |
| Always-on capture | Windows Recall | An unencrypted database readable by any process; withdrawn [W-55][W-57] | No capture without a holding and a visible indicator |
| Periodic re-consent | macOS screen recording | Loops and fatigue [W-103][W-104] | Persistent grants, live indicators, a log |
| Multikernel as scalability | Barrelfish | Core counts did not explode; capabilities for memory too complex [A-57] | Not relevant before SMP; noted for Phase 12 |

---

## 9. What this pass changes about the design

The pass did not overturn any accepted decision. It supplied the evidence for
the userland design in `019-userland.md` and the style guide in
`020-style-guide.md`, and it settled the questions in section 0. Three findings
deserve a sentence each because they shaped the design more than the rest.

First, the three traditions that took capability security seriously in a user
interface (Polaris and the powerbox, Access Control Gadgets, Sculpt's graph) all
arrived at the same rule from different directions: derive authority from what
the user is already doing and show the graph, never ask. The kernel's one-handle
birth and rootless namespace (D-111, D-171) make that rule the natural user
model rather than a policy layer on top of one.

Second, every successful terminal is extending the VT model toward records, and
the critics of VT are right about the cause. A new OS whose native terminal is a
record protocol, with VT in a compatibility component for ports, is the position
both camps would accept.

Third, the strip: niri is the first tiling model that never resizes what is
being read, COSMIC made stacks a compositor primitive, and Windows' Snap
fractions and taskbar are what a Windows-leaning user will not give up. The
combination is not shipped anywhere, which is a reason to write the design down
carefully and to keep the floating mode one key away.
