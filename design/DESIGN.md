---
name: Fluent Desktop ADB Toolkit
colors:
  surface: '#131313'
  surface-dim: '#131313'
  surface-bright: '#393939'
  surface-container-lowest: '#0e0e0e'
  surface-container-low: '#1b1c1c'
  surface-container: '#202020'
  surface-container-high: '#2a2a2a'
  surface-container-highest: '#353535'
  on-surface: '#e5e2e1'
  on-surface-variant: '#bdc8cf'
  inverse-surface: '#e5e2e1'
  inverse-on-surface: '#303030'
  outline: '#889299'
  outline-variant: '#3e484e'
  surface-tint: '#74d1ff'
  primary: '#b4e4ff'
  on-primary: '#003548'
  primary-container: '#60cdff'
  on-primary-container: '#005572'
  inverse-primary: '#006687'
  secondary: '#a3c9ff'
  on-secondary: '#00315c'
  secondary-container: '#3b93f1'
  on-secondary-container: '#002a51'
  tertiary: '#ffd3d4'
  on-tertiary: '#66041a'
  tertiary-container: '#ffabaf'
  on-tertiary-container: '#912836'
  error: '#ffb4ab'
  on-error: '#690005'
  error-container: '#93000a'
  on-error-container: '#ffdad6'
  primary-fixed: '#c1e8ff'
  primary-fixed-dim: '#74d1ff'
  on-primary-fixed: '#001e2b'
  on-primary-fixed-variant: '#004d67'
  secondary-fixed: '#d3e3ff'
  secondary-fixed-dim: '#a3c9ff'
  on-secondary-fixed: '#001c39'
  on-secondary-fixed-variant: '#004883'
  tertiary-fixed: '#ffdada'
  tertiary-fixed-dim: '#ffb3b6'
  on-tertiary-fixed: '#40000c'
  on-tertiary-fixed-variant: '#861f2f'
  background: '#131313'
  on-background: '#e5e2e1'
  surface-variant: '#353535'
typography:
  headline-lg:
    fontFamily: Geist
    fontSize: 1.75rem
    fontWeight: '600'
    lineHeight: 2.25rem
    letterSpacing: -0.02em
  headline-md:
    fontFamily: Geist
    fontSize: 1.25rem
    fontWeight: '600'
    lineHeight: 1.75rem
    letterSpacing: -0.015em
  headline-sm:
    fontFamily: Geist
    fontSize: 1rem
    fontWeight: '600'
    lineHeight: 1.5rem
    letterSpacing: -0.01em
  body-lg:
    fontFamily: Geist
    fontSize: 0.9375rem
    fontWeight: '400'
    lineHeight: 1.375rem
  body-md:
    fontFamily: Geist
    fontSize: 0.8125rem
    fontWeight: '400'
    lineHeight: 1.25rem
  body-sm:
    fontFamily: Geist
    fontSize: 0.75rem
    fontWeight: '400'
    lineHeight: 1.125rem
  label-md:
    fontFamily: Geist
    fontSize: 0.8125rem
    fontWeight: '500'
    lineHeight: 1.125rem
  label-sm:
    fontFamily: Geist
    fontSize: 0.6875rem
    fontWeight: '500'
    lineHeight: 1rem
    letterSpacing: 0.02em
  code-terminal:
    fontFamily: JetBrains Mono
    fontSize: 0.75rem
    fontWeight: '400'
    lineHeight: 1.25rem
  code-inline:
    fontFamily: JetBrains Mono
    fontSize: 0.8125rem
    fontWeight: '500'
    lineHeight: 1.125rem
rounded:
  sm: 0.125rem
  DEFAULT: 0.25rem
  md: 0.375rem
  lg: 0.5rem
  xl: 0.75rem
  full: 9999px
spacing:
  gutter: 0.75rem
  margin: 1rem
  space-xs: 0.25rem
  space-sm: 0.5rem
  space-md: 0.75rem
  space-lg: 1rem
  space-xl: 1.5rem
---

## Brand & Style

This design system targets power users, Android TV developers, and smart hardware enthusiasts operating within a high-performance native desktop environment. Built on the visual vernacular of modern Windows 11 Fluent 2 Design, the interface evokes precision, responsiveness, and native OS cohesion.

The visual mood blends precision desktop utility with deep slate minimalism and subtle tactile feedback. It avoids cluttered enterprise tool bloat in favor of clean segmentation: frosted backdrop surfaces, crisp hairline segmentations, purposeful telemetry, and micro-interactions tuned for high-density workflow throughput.

## Colors

The color palette embraces Windows 11 dark mode hierarchies:
- **Primary (`#60CDFF`)**: Fluent Accent Light, used for high-visibility highlights, active navigation markers, selection pills, and primary interactive elements against deep dark backgrounds.
- **Secondary (`#0078D4`)**: Fluent Accent Default, leveraged for focused borders, brand anchors, and elevated action states.
- **Tertiary (`#F1707B`)**: Subtle soft crimson used for destructive actions (e.g., Force Stop, Disconnect, Uninstall, Danger badges) and terminal error alerts.
- **Neutral (`#202020`)**: Deep slate foundation.

### Surface and Hierarchy Roles
- **Mica Canvas Background**: `#1F1F1F` with 85% opacity over subtle desktop backdrop blurs.
- **Sidebar & Chrome**: `#202020` providing quiet contrast against the primary workspace.
- **Card & Container Layer**: `#2C2C2C` with subtle 1px border `#3A3A3A` (or `rgba(255, 255, 255, 0.07)`).
- **Subtle Fill / Hover**: `rgba(255, 255, 255, 0.05)` to `rgba(255, 255, 255, 0.09)`.
- **Text & Foreground**:
  - Primary Text: `#FFFFFF` (90% opacity / `#E4E4E4`)
  - Secondary Text: `#A0A0A0` (60% opacity)
  - Disabled Text: `#5E5E5E`
- **Telemetry & Status**:
  - Connected / Online: `#6CCB5F`
  - Warning / Debugging Off: `#FCE100`
  - Disconnected / Standby: `#8A8A8A`

## Typography

The typography is built around clean, neutral geometric sans glyphs matching desktop OS specifications, complemented by a developer-grade monospaced face for shell output, memory allocation, and package names.

- **Primary UI (Headlines, Body, Navigation)**: Rendered in `Geist`, mirroring the clean geometric precision and legibility of desktop interface guidelines.
- **ADB Logs & Hardware Identifiers**: Rendered in `JetBrains Mono` for aligned columnar readout of device telemetry, ports, serial numbers, and shell streams.

## Layout & Spacing

The layout operates on a desktop-native shell architecture structured into three persistent panes:
1. **Title Bar Chrome (36px fixed height)**: Houses window management controls, integrated application icon, breadcrumb device title, and minimize/maximize/close actions.
2. **Left Master Navigation / Device Panel (280px fixed width)**: Houses IP address scan inputs, cached/active TV targets, connection status cards, and pairing wizard toggles.
3. **Workspace Core (Dynamic Flex)**: Houses the Fluent horizontal pivot bar (`Packages`, `Performance`, `Install APK`, `Settings`, etc.), toolbars, high-density data views, and a collapsible bottom ADB console drawer (default 180px, resizable up to 400px).

Margins prioritize compact desktop data density without crowding interactive touch targets. Gutters between cards and control fields are maintained at `0.75rem` (`12px`) to accommodate rapid keyboard-and-mouse scanning.

## Elevation & Depth

Visual hierarchy uses Windows 11 acrylic and Mica principles, relying on translucent layers, low-opacity internal borders, and subtle diffuse shadows:

- **Level 0 (Canvas / Window Root)**: Mica baseline with `#1F1F1F` background and 16px backdrop blur.
- **Level 1 (Docked Shells & Side Rail)**: `#202020` with a 1px border (`rgba(255, 255, 255, 0.06)`) on the right and bottom edges.
- **Level 2 (Cards, Interactive Surfaces, Terminal Drawer)**: `#2C2C2C` with subtle top highlight border (`rgba(255, 255, 255, 0.09)`) and bottom shadow `0 2px 4px rgba(0, 0, 0, 0.25)`.
- **Level 3 (Flyouts, Popovers, Context Menus)**: Acrylic surface (`#2A2A2A` at 92% opacity with 24px blur) accompanied by an ambient drop shadow: `0 8px 24px rgba(0, 0, 0, 0.45)` bounded by a 1px contour stroke (`rgba(255, 255, 255, 0.12)`).

## Shapes

The design uses soft, subtle rounding aligned with desktop Fluent standards:
- **Default Buttons, Text Fields, Segmented Pills**: `0.25rem` (`4px`) to `0.375rem` (`6px`).
- **Cards, Terminal Drawers, List Item Highlights**: `0.375rem` (`6px`) to `0.5rem` (`8px`).
- **Window Shell Outer Framing**: `0.5rem` (`8px`) on Windows 11 window boundaries.
- **Status Indicators & Badge Dots**: Circular (`9999px`).

## Components

### Title Bar & Window Chrome
- **Height**: 36px non-client area with seamless Mica tinting.
- **Window Buttons**: Native minimize, maximize/restore, and close buttons on the upper right; close button transitions to `#C42B1C` with white glyph on hover.

### Side Navigation & Device Selector
- **Device Row Card**: 52px height, containing a status dot indicator (Green for online, Amber for offline/debugging off), device display name, IP address, and an inline unpair action.
- **Active State**: Selected device card takes an acrylic tint `rgba(255, 255, 255, 0.08)` accented with an absolute left border pill (3px width, `#60CDFF`, rounded).
- **IP Connect Module**: Input fields with monospaced text for IP and Port (`5555`), paired with high-contrast connect/disconnect buttons.

### Fluent Tab Bar (Pivot Navigation)
- Horizontal layout across the top of the main pane: *Packages*, *Performance*, *Install*, *Settings*, *Tools*, *Media*, *Remote*, *Screenshot*.
- Unselected tabs use muted `#A0A0A0` text; active tab renders with `#FFFFFF` text and a rounded background pill (`rgba(255, 255, 255, 0.08)`) or bottom accent underline (`#60CDFF`).

### Action Buttons & Command Bar
- **Primary Action (e.g., Save APK, Connect)**: Solid `#60CDFF` background, `#000000` text, medium weight, subtle internal highlight on hover (`#78D5FF`).
- **Standard Action (e.g., Clear Cache, Restore)**: Surface `#2C2C2C`, text `#E4E4E4`, border `1px solid rgba(255, 255, 255, 0.08)`; on hover, background brightens to `#353535`.
- **Destructive Action (e.g., Disconnect, Uninstall, Force Stop)**: Surface tinted with `#3A1E22`, border `1px solid rgba(241, 112, 123, 0.3)`, text `#F1707B`; on hover, background transitions to `#48242A`.

### High-Density Package Data Grid
- Row height: 38px compact or 48px detailed.
- Columns: Checkbox, App Icon (24x24px), App Name, Package ID (monospaced), Version, Size, Type Tag (System, User, Sideloaded), Action Menu button.
- Zebra striping is avoided; rows rely on subtle hover highlight `rgba(255, 255, 255, 0.04)`.

### Expandable ADB Log Terminal Drawer
- Docked at bottom of the main viewport with a 1px top border (`#333333`) and drag-to-resize handle.
- Header bar includes title, copy button, clear log button, and an auto-scroll checkbox.
- Body uses monospaced text (`JetBrains Mono`, 12px) over `#181818` with color-coded syntax output:
  - Timestamps: `#5E5E5E`
  - Info: `#E4E4E4`
  - ADB commands: `#60CDFF`
  - Success: `#6CCB5F`
  - Errors/Warnings: `#F1707B`