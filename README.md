# Spectre Desktop Environment

> **A lightweight, dark and highly customizable Linux desktop environment with a focus on performance, fluid workspaces and subtle cyber-inspired visuals.**

**Status:** Concept / early development  
**Primary platform:** Linux / Wayland  
**Reference distribution:** Garuda Linux  
**Goal:** Run well on both modern gaming/workstation systems and older low-spec hardware.

---

## Overview

**Spectre DE** is a planned desktop environment for Linux inspired by the flexibility of KDE Plasma and the low resource usage of XFCE.

The visual language is primarily **black and minimal**. Instead of excessive neon effects, Spectre uses subtle RGB accents and animated **topographic/contour-line patterns** as a recognizable design element.

Spectre is intended to be distribution-independent. **Garuda Spectre** will be the Garuda Linux configuration/theme, while the core desktop should eventually be usable on Arch Linux, Debian, Ubuntu, Fedora and other Linux distributions.

## Design goals

- **Low RAM and CPU usage** — suitable for old laptops and low-end systems.
- **Modern Wayland desktop** — designed around a native Wayland compositor.
- **Dark by default** — black is the dominant UI color.
- **Subtle RGB** — color is used as an accent, not as visual noise.
- **Spectre Pattern** — animated contour-line patterns inspired by topographic maps.
- **Fluid workspaces** — smooth and optional 3D workspace transitions.
- **Highly customizable** — panel layout, patterns, animations, colors and effects can be changed.
- **Performance scaling** — expensive visual effects can be disabled independently.
- **Distribution independent** — no hard dependency on Garuda-specific components.
- **Cybersecurity-oriented workflow** — efficient keyboard navigation, terminal-friendly design and useful system information without turning the desktop into a gimmick.

---

## Planned architecture

```text
spectre-desktop
│
├── spectre-compositor   # Wayland compositor and window management
├── spectre-panel        # Taskbar / panel
├── spectre-shell        # Desktop shell and integration
├── spectre-launcher     # Application launcher
├── spectre-settings     # Central settings application
├── spectre-notify       # Notification daemon / UI
├── spectre-lock         # Lock screen
├── spectre-session      # Session startup and management
├── spectre-effects      # Optional visual effects and shaders
└── spectre-theme        # Default Spectre visual assets
```

Spectre should reuse established Linux infrastructure wherever possible instead of reinventing it.

```text
                     Spectre DE
                         │
          ┌──────────────┼──────────────┐
          │              │              │
      Compositor       Panel          Shell
          │              │              │
          └──────────────┼──────────────┘
                         │
                      Wayland
                         │
        ┌────────────────┼────────────────┐
        │                │                │
     PipeWire          BlueZ       NetworkManager
      Audio           Bluetooth       Network
```

## Spectre Panel

The panel is one of the main visual identities of Spectre.

The default design should stay close to the usability of a traditional KDE-style taskbar while improving its appearance and workspace interaction.

Planned features include:

- Application launcher
- Pinned and running applications
- System tray
- Clock and date
- Audio, network, Bluetooth and power controls
- Virtual desktop/workspace indicator
- Optional CPU, RAM and network information
- Modular/reorderable widgets
- Floating or full-width modes
- Optional transparency
- Subtle contour-line background
- Small RGB accent for the active workspace/application

The panel should remain functional with **all animations and RGB effects disabled**.

---

## Workspace experience

Workspace switching is intended to become one of Spectre's signature features.

Instead of a GNOME-style overview being required for every switch, Spectre will provide fast normal switching plus optional visual transitions such as:

- Slide
- Depth
- Cube / 3D
- Coverflow-inspired transition
- Minimal fade
- No animation

Effects must never be mandatory. Users on old hardware should be able to disable them completely.

## Spectre Pattern

A recurring contour/topographic-line pattern will appear subtly in areas such as:

- Window title bars
- Panel backgrounds
- Workspace transitions
- Launcher
- Lock screen
- Optional desktop background

Example settings concept:

```text
Appearance → Spectre Pattern

Pattern                 Topographic
Animation               On / Off
Animation speed         ━━━━━●━━
RGB accents             On / Off
RGB intensity           ━━●━━━━━
Window decorations      On / Off
Panel pattern           On / Off
Desktop pattern         On / Off
```

When animation is disabled, the pattern should become static rather than disappear.

---

## Performance profiles

Spectre should offer simple presets while still allowing individual settings to be changed.

### Performance

For old laptops, VMs and low-power hardware.

- Static patterns
- Minimal shadows
- Simple workspace transitions
- No blur
- No animated RGB
- No 3D workspace effects

### Balanced

The intended default.

- Subtle animated patterns
- Lightweight RGB accents
- Smooth workspace transitions
- Limited transparency/effects

### Spectre

For modern hardware.

- Animated contour patterns
- RGB window accents
- Blur and transparency
- 3D workspace transitions
- Enhanced window animations

Performance profiles must only affect visual features — **never basic desktop functionality**.

---

## Technology direction

The exact stack is not final yet, but the current direction is:

- **Wayland** as the primary display protocol
- **Rust and/or C++** for performance-critical components
- A lightweight compositor foundation rather than implementing every Wayland protocol from scratch
- **Qt 6** where it provides a clear advantage for configuration and desktop UI
- GPU shaders for optional animated patterns and effects
- IPC between compositor, panel and shell
- Standard freedesktop.org protocols wherever possible

X11 compatibility may be provided through **XWayland** rather than maintaining a separate X11 desktop implementation.

## Distribution support

The long-term goal is native packages for multiple distributions.

```text
Arch Linux / Garuda     spectre-desktop package
Debian / Ubuntu         spectre-desktop .deb
Fedora                   spectre-desktop .rpm
```

After installation, Spectre should appear as a normal session in compatible display managers alongside Plasma, GNOME, XFCE and other environments.

## What Spectre will NOT reinvent

Spectre does not need its own implementation of every Linux subsystem.

Existing projects can provide functionality such as:

- Networking — NetworkManager
- Audio — PipeWire / WirePlumber
- Bluetooth — BlueZ
- Authentication — Polkit
- X11 application compatibility — XWayland

A dedicated Spectre file manager, terminal or other applications may be considered later, but they are **not required for the first desktop release**.

---

## Roadmap

### Phase 0 — Design

- [x] Define visual direction
- [x] Define initial panel concepts
- [x] Define Spectre Pattern concept
- [ ] Finalize UI design system
- [ ] Define RAM/CPU performance targets
- [ ] Choose compositor foundation and primary language

### Phase 1 — Panel prototype

- [ ] Create `spectre-panel`
- [ ] Application launcher button
- [ ] Workspace indicator
- [ ] Pinned/running applications
- [ ] System tray
- [ ] Clock
- [ ] Configuration file
- [ ] Static Spectre Pattern

### Phase 2 — Core desktop

- [ ] Create compositor prototype
- [ ] Window management
- [ ] Keyboard shortcuts
- [ ] Multi-monitor support
- [ ] Shell integration
- [ ] Notifications
- [ ] Session management

### Phase 3 — Spectre visuals

- [ ] RGB window decorations
- [ ] Animated contour patterns
- [ ] Workspace animations
- [ ] 3D workspace effects
- [ ] Performance profiles
- [ ] Animation kill-switch

### Phase 4 — Distribution

- [ ] Arch package
- [ ] Garuda package/repository
- [ ] Debian/Ubuntu package
- [ ] Fedora package
- [ ] Installation documentation

---

## Project principles

1. **Performance before effects.**
2. **Every expensive animation must be optional.**
3. **Black first, RGB second.**
4. **Do not sacrifice usability for the cyber aesthetic.**
5. **Use Linux standards instead of unnecessary custom replacements.**
6. **Old hardware is a supported target, not an afterthought.**
7. **Spectre must remain usable without any visual effects.**

## Current status

Spectre DE is currently a **design and architecture concept**. There is no production-ready desktop environment yet.

The first planned implementation target is **Spectre Panel**, followed by the core Wayland compositor and shell components.

## Naming

- **Project:** Spectre Desktop Environment
- **Short name:** Spectre DE
- **Garuda configuration/theme:** Garuda Spectre

This separation allows Spectre to remain usable outside Garuda Linux while retaining a dedicated Garuda experience.

---

## License

**TBD.** An open-source license will be selected before the first public source release.

---

<p align="center">
  <strong>SPECTRE DE</strong><br>
  Lightweight. Dark. Fluid.
</p>
