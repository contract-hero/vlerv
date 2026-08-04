---
name: Vlervtifacts
description: Warm ink-and-paper chrome for a macOS artifact reading room — one ember accent, no cool grey.
colors:
  accent: "#dda355"
  accent-strong: "#e8b56d"
  bg: "#1c1a17"
  bg-sidebar: "#100e0c"
  bg-elevated: "#292520"
  bg-row-hover: "#26221d"
  bg-row-selected: "#322b21"
  border: "#413a31"
  fg: "#d8d2c6"
  fg-muted: "#968e7f"
  fg-dim: "#6b6458"
  fg-path: "#a49b8a"
  heading-fg: "#ece6da"
  label-active-fg: "#f4efe6"
  code-bg: "#262219"
  code-fg: "#e3c893"
  pre-bg: "#141210"
  input-bg: "#0c0a08"
  input-border: "#3b352c"
  button-hover-bg: "#3a342b"
  badge-bg: "#322b21"
  badge-fg: "#dda355"
  error-fg: "#e08a7a"
  star-idle: "#4d463c"
  scrollbar-thumb: "#363028"
typography:
  display:
    fontFamily: "\"New York\", ui-serif, Georgia, serif"
    fontSize: "34px"
    fontWeight: 600
    lineHeight: 1.1
    letterSpacing: "-0.01em"
  headline:
    fontFamily: "-apple-system, BlinkMacSystemFont, \"SF Pro Text\", \"Segoe UI\", system-ui, sans-serif"
    fontSize: "2em"
    fontWeight: 600
    lineHeight: 1.25
    letterSpacing: "normal"
  title:
    fontFamily: "-apple-system, BlinkMacSystemFont, \"SF Pro Text\", \"Segoe UI\", system-ui, sans-serif"
    fontSize: "15px"
    fontWeight: 600
    lineHeight: 1.3
    letterSpacing: "normal"
  body:
    fontFamily: "-apple-system, BlinkMacSystemFont, \"SF Pro Text\", \"Segoe UI\", system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.4
    letterSpacing: "normal"
  reading:
    fontFamily: "-apple-system, BlinkMacSystemFont, \"SF Pro Text\", \"Segoe UI\", system-ui, sans-serif"
    fontSize: "15.5px"
    fontWeight: 400
    lineHeight: 1.65
    letterSpacing: "normal"
  label:
    fontFamily: "-apple-system, BlinkMacSystemFont, \"SF Pro Text\", \"Segoe UI\", system-ui, sans-serif"
    fontSize: "11px"
    fontWeight: 700
    lineHeight: 1.2
    letterSpacing: "0.06em"
  mono:
    fontFamily: "\"SF Mono\", ui-monospace, SFMono-Regular, Menlo, monospace"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.55
    letterSpacing: "normal"
rounded:
  sm: "4px"
  md: "6px"
  lg: "10px"
spacing:
  "1": "4px"
  "2": "8px"
  "3": "12px"
  "4": "16px"
  "5": "24px"
  "6": "32px"
components:
  tab:
    backgroundColor: "transparent"
    textColor: "{colors.fg-muted}"
    typography: "{typography.mono}"
    padding: "0 8px 0 12px"
    height: "38px"
  tab-active:
    backgroundColor: "{colors.bg}"
    textColor: "{colors.fg}"
    typography: "{typography.mono}"
    padding: "0 8px 0 12px"
    height: "38px"
  toolbar-button:
    backgroundColor: "transparent"
    textColor: "{colors.fg-muted}"
    rounded: "{rounded.md}"
    height: "28px"
    width: "28px"
  toolbar-button-hover:
    backgroundColor: "{colors.bg-row-hover}"
    textColor: "{colors.fg}"
    rounded: "{rounded.md}"
    height: "28px"
    width: "28px"
  address-input:
    backgroundColor: "{colors.input-bg}"
    textColor: "{colors.fg}"
    typography: "{typography.mono}"
    rounded: "{rounded.md}"
    padding: "5px 10px"
  explorer-row:
    backgroundColor: "transparent"
    textColor: "{colors.fg}"
    padding: "0 12px 0 0"
    height: "30px"
  explorer-row-selected:
    backgroundColor: "{colors.bg-row-selected}"
    textColor: "{colors.label-active-fg}"
    padding: "0 12px 0 0"
    height: "30px"
  start-action:
    backgroundColor: "{colors.bg-sidebar}"
    textColor: "{colors.fg}"
    rounded: "{rounded.md}"
    padding: "7px 12px"
  sidebar-button:
    backgroundColor: "{colors.bg-row-selected}"
    textColor: "{colors.fg}"
    rounded: "{rounded.md}"
    padding: "8px 14px"
  context-menu-item:
    backgroundColor: "transparent"
    textColor: "{colors.fg}"
    rounded: "5px"
    padding: "6px 10px"
  badge-external:
    backgroundColor: "{colors.badge-bg}"
    textColor: "{colors.badge-fg}"
    rounded: "{rounded.sm}"
    padding: "2px 6px"
---

# Design System: Vlervtifacts

## Overview

**Creative North Star: "The Archivist's Desk"**

This is not a browser and not an editor. It is the desk of someone who keeps
things: a workspace tree that holds a collection, bookmarks and recents that
record what mattered, a quick-open palette that retrieves on demand, and a
reading surface where the held object is finally examined. Custody comes
first, reading second. Every surface in the chrome exists to fetch, hold, or
hand over an artifact, and then to get out of the way while it is read.

The material is warm. Neutrals carry a low warm hue in both themes — an ink
palette (`#1c1a17` upward) after dark, a paper palette (`#faf8f4` downward) in
daylight — and the two follow the macOS system appearance without a manual
switch. A single ember accent is the only chromatic voice in the product. It
marks what is active, what is held, and where the keyboard is. Nothing else in
the interface is allowed to be colorful.

Density is deliberate. Base UI type is 13px, rows are 30px, chrome bands are
38–40px tall, and the radius scale stops at 10px. The system reads as crafted,
archival and patient: built to be lived in for hours by one person who knows
every shortcut, not to impress on first sight. Confirmed rejections: cool
editor blue-grey, glassmorphism and backdrop blur, neon or high-saturation
accents, and rounded consumer-app playfulness.

**Key Characteristics:**

- Warm neutrals only; no cool grey anywhere in the chrome.
- One accent, used as a marker and never as decoration.
- Dual ink/paper themes driven by the OS, with the same structure in both.
- High density: 13px UI type, 30px rows, 4px spacing grid.
- Native macOS posture: overlay title bar, system fonts, system appearance.
- The artifact viewport is the largest and quietest region on screen.

## Colors

Every neutral is warm-shifted, and exactly one hue carries meaning.

### Primary

- **Warm Ember** (`#dda355` ink / `#9a6a1e` paper): the whole color story. It
  appears as the 2px underline on the active tab, the 2px inset edge on a
  selected tree row, every `:focus-visible` ring, active bookmark stars, the
  sidebar resizer on hover and drag, the drop-target edge while reordering
  tabs and bookmarks, the input border on focus, Markdown link text, the
  blockquote rule, and the pulsing tab loading dot. Nowhere else.
- **Ember Bright** (`#e8b56d` ink / `#7d5517` paper): the hover state of an
  ember-colored element, mainly Markdown links. It is a state, not a second
  accent.

### Neutral

The ink theme is the default and doubles as the pre-hydration fallback, so no
frame renders with unstyled values.

- **Deep Ink** (`#1c1a17`): the reading field — the preview pane and window
  ground.
- **Shadowed Ink** (`#100e0c`): recessed chrome — sidebar, tab strip, toolbar.
  Darker than the reading field, so the chrome sits behind the artifact.
- **Raised Ink** (`#292520`): detached surfaces — context menus, quick open,
  the notice toast.
- **Hover Ink** (`#26221d`) and **Held Ink** (`#322b21`): the two row states.
  Held Ink also backs badges and the one filled button.
- **Rule Ink** (`#413a31`): every 1px divider, border and table cell edge.
- **Parchment** (`#d8d2c6`): body text.
- **Faded Parchment** (`#968e7f`): secondary text, inactive tab labels, icon
  buttons at rest.
- **Ghost Parchment** (`#6b6458`): tertiary text, chevrons, keyboard hints,
  footer text.
- **Path Parchment** (`#a49b8a` ink / `#655e50` paper): filesystem paths in
  retrieval surfaces — the start page and quick open. Its own token because a
  path is the text that tells two same-named artifacts apart, and it must
  clear 4.5:1 on every surface it lands on, including Raised Ink and Held Ink.
- **Bleached Parchment** (`#ece6da` headings / `#f4efe6` active row labels):
  the brightest values in the system, reserved for headings and the selected
  row.

### Tertiary

- **Signal Coral** (`#e08a7a` ink / `#b5493a` paper): failure only — read
  errors, the file-deleted notice, destructive menu items, the remove-bookmark
  hover. It is never used for emphasis.
- **Ember Code** (`#e3c893` ink / `#7a5218` paper): inline code text on
  `code-bg`. Related to the accent by hue, deliberately weaker, so a paragraph
  full of inline code does not read as a paragraph full of links.

### Named Rules

**The Single Lamp Rule.** There is one accent hue in the product. If a new
element needs to stand out, it earns tone, weight, or position — not a second
color. A screen showing more than a few ember marks at once has lost the
metaphor.

**The Warm Neutral Rule.** No neutral in this system is hue-neutral or cool.
Every grey carries a warm cast. A `#2a2a2a` or a `#8892a0` anywhere in the
chrome is a defect, not a variation.

**The Coral-Means-Broken Rule.** Coral is reserved for something that failed
or will be destroyed. It never marks emphasis, novelty, or a required field.

**The Glyphs Carry Shape Rule.** File-type icons encode type through their
drawn shape and never through hue. Every glyph renders in `currentColor` and
inherits the tone of its row. Exactly one tonal distinction is allowed: the
artifacts this app exists to read (`.html`, `.md`) sit one step brighter than
everything else. A per-language icon palette is the fastest way to lose the
Single Lamp Rule on every surface at once.

## Typography

**Display Font:** New York (with ui-serif, Georgia fallback)
**Body Font:** system UI — `-apple-system` / SF Pro Text (with Segoe UI, system-ui fallback)
**Label/Mono Font:** SF Mono (with ui-monospace, SFMono-Regular, Menlo fallback)

**Character:** The interface speaks in the system voice — a native macOS app
that does not announce a typeface. The serif appears exactly once, on the
start-page wordmark, where the product finally says its own name. Monospace
carries every filesystem path, which makes paths scannable and visually
separates machine truth from human labels.

### Hierarchy

- **Display** (600, 34px, -0.01em, New York serif): the start-page wordmark.
  One instance in the whole product.
- **Headline** (600, 2em of reading size ≈ 31px, 1.25): Markdown `h1` in
  rendered artifacts, with a 1px bottom rule. `h2` follows at 1.5em, also
  ruled; `h3` at 1.25em, unruled.
- **Title** (600, 15px, 1.3): in-app notice headings, such as the file-deleted
  state.
- **Body** (400, 13px, 1.4): the app's own UI text and the root font size.
  Tabs and sidebar rows step down to 12px; explorer rows step up to 14px.
- **Reading** (400, 15.5px, 1.65): rendered Markdown body text, in a 900px
  measure. This is the only place in the product where comfort outranks
  density.
- **Label** (700, 11px, 0.06em, uppercase): section headers — the sidebar
  workspace header, start-page section titles. Bookmarks step down to 10px,
  as do `kbd` chips and the quick-open footer hint.
- **Mono** (400, 12px, 1.55): the address bar, every path, the start-page
  subtitle, code, and preformatted text. Code blocks render at 13.5px.

### Named Rules

**The One Serif Moment Rule.** New York appears on the wordmark and nowhere
else. A second serif element dilutes the only moment the product speaks in its
own voice.

**The Paths Are Mono Rule.** Anything that is a filesystem path renders in
`mono`, right-aligned and dimmed when it accompanies a filename. A path in the
UI font is a defect.

**The Density Floor Rule.** Chrome type does not go below 10px, and 11px
uppercase labels always carry 0.06em tracking. Small type without tracking is
unreadable at this density.

## Layout

The window is a two-pane horizontal split: a sidebar of fixed width and a
preview pane taking the remainder. The sidebar clamps between 200px and 480px,
carries its width in persisted state, and is dragged by a 6px hit-target
resizer that is invisible until hovered. The preview pane stacks three fixed
chrome bands above a scrolling viewport: tab strip (38px), toolbar (40px), and
an optional notice strip, then the artifact.

Spacing is a 4px grid running 4 / 8 / 12 / 16 / 24 / 32px. Chrome padding
lives in the 8–12px range; content padding in the 16–32px range. Rows in the
tree are 30px tall and indent by a fixed step per depth level.

Content measures are capped, not fluid. Rendered Markdown centers at 900px
with 32px/28px padding. The start page centers at 560px with a 56px top inset
that clears the traffic lights. Quick open is `min(560px, 86vw)` at 12vh from
the top, capped at 60vh. Context menus start at 180px. Tabs run 110–220px and
scroll horizontally with the scrollbar suppressed.

There are no responsive breakpoints. The only elastic axis is the sidebar
split, and the window enforces a 640×400 minimum. Design for a resizable
desktop window, not for viewport classes.

### Named Rules

**The Aligned Band Rule.** The chrome is two bands, and both run unbroken
across the whole window. The sidebar's drag strip and the tab strip are both
38px; the sidebar header and the toolbar are both 40px. Any new top-level
chrome band matches one of those heights or explicitly sits below the line.

**The Artifact Gets the Rest Rule.** Chrome takes fixed bands; the artifact
takes every remaining pixel. No chrome element grows with the window.

## Elevation & Depth

The system is layered and lifted. Depth is built primarily from tonal steps —
Shadowed Ink chrome behind the Deep Ink reading field, Hover Ink and Held Ink
above that, Raised Ink for detached surfaces — with 1px warm rules marking
every plane change. Shadow is the second instrument, reserved for surfaces
that leave the layout entirely.

The shipped vocabulary is deliberately small: two tokens, both warm-black
rather than neutral-black, so a shadow never reads as cool grey over a warm
ground. New elevation extends this ramp rather than inventing one-off values.

### Shadow Vocabulary

- **Menu lift** (`box-shadow: 0 8px 28px rgba(12, 9, 5, 0.45)`; paper:
  `rgba(60, 48, 30, 0.18)`): context menus — a surface that appeared at the
  pointer and will vanish.
- **Overlay lift** (`box-shadow: 0 16px 48px rgba(12, 9, 5, 0.55)`; paper:
  `rgba(60, 48, 30, 0.25)`): modal overlays such as quick open, over a
  `rgba(10, 8, 5, 0.4)` scrim.

### Named Rules

**The Earned Lift Rule.** Lift responds to detachment or state. A surface that
sits in the layout is flat and separated by tone and a 1px rule; a surface
that floats above the layout casts a warm shadow. Decorative resting shadows
are not part of this system.

**The Warm Shadow Rule.** Shadow color is warm-black, never `rgba(0,0,0,x)`.
A neutral shadow over warm ink reads as a cool bruise.

## Shapes

Rectilinear and tight. The radius scale is 4 / 6 / 10px: 4px for small inline
marks (badges, code, focus rings, close buttons), 6px for interactive controls
(toolbar buttons, inputs, start-page rows and actions, list items), and 10px
for detached surfaces (context menu, quick-open panel). Menu items sit at 5px
and the scrollbar thumb at 5px on a 10px track.

Borders are always 1px in Rule Ink. Separation is a hairline, never a heavy
frame or a double rule. The one curved exception is the 6px tab loading dot,
a full circle because it is a pulse indicator, not a container.

Active state is expressed as a 2px straight bar, never a fill or an outline:
across the top of the active tab, inset on the leading edge of a selected row,
inset on the leading edge of a tab drop target, and inset on the top edge of a
bookmark drop target.

### Named Rules

**The 10px Ceiling Rule.** Nothing exceeds a 10px radius. No pills, no capsule
buttons, no circular avatars. The desk is made of rectangles.

**The 2px Marker Rule.** Selection and activity are marked by a 2px ember bar
on an edge. Filling a whole element with the accent is not how this system
says "active".

## Components

Components read as solid and substantial: real surfaces with visible edges and
weight, not floating text targets. Every interactive element has a resting
tone, a hover tone, and a visible keyboard focus ring; icon-only controls
reveal themselves on row hover rather than sitting permanently lit.

### Buttons

- **Shape:** softly squared (6px), or small-squared (4px) for 16–18px icon
  buttons.
- **Primary (`.button`):** Held Ink surface, 1px Rule Ink border, 500
  weight, 8px/14px padding; hovers to Button Hover Ink (`#3a342b`).
- **Icon (toolbar):** 28×28px, transparent at rest, Faded Parchment glyph;
  hovers to Hover Ink with a Parchment glyph. Disabled drops to Ghost
  Parchment at 0.5 opacity with a default cursor.
- **Ghost (start-page action):** Shadowed Ink surface, 1px Rule Ink border,
  7px/12px padding, an inline 14px icon, and an optional `kbd` chip. On hover
  the surface lifts to Hover Ink and the border warms to Ghost Parchment.
- **Hover / Focus:** background and color transition over 80ms
  (`cubic-bezier(0.2, 0, 0.2, 1)`). Focus is a 2px ember ring inset by 2px.

### Cards / Containers

- **Corner Style:** 10px for detached panels; the layout panes are unrounded
  and meet at hairlines.
- **Background:** Raised Ink for detached panels, Shadowed Ink for chrome.
- **Shadow Strategy:** menu lift or overlay lift; see Elevation & Depth.
- **Border:** 1px Rule Ink on every panel, including shadowed ones.
- **Internal Padding:** 4px for list-shaped panels, 10–12px for input rows.

### Inputs / Fields

- **Style:** Input Ink (`#14120f`) — darker than the surrounding chrome, so
  the field reads as a recess — with a 1px `#332e26` border, 6px radius,
  5px/10px padding, monospace text.
- **Focus:** the border becomes ember over 80ms. The native outline is
  suppressed here because the border shift is the focus signal.
- **Error:** a floating tip below the field on Raised Ink with a 1px border
  and Signal Coral 11px text. The field itself does not change color.
- **Borderless variant:** the quick-open search input has no chrome at all;
  the panel is the field.

### Navigation

- **Tabs:** 38px tall, 110–220px wide, 12px type, Faded Parchment label with a
  12px file icon and a close button that appears on hover or when active. The
  active tab takes the Deep Ink background of the content below it — it
  becomes part of the reading field — plus a 2px ember bar across its top.
  Dragging drops the source to 0.4 opacity and marks the target with a 2px
  ember inset edge.
- **Tree rows:** 30px tall, 14px type, a rotating chevron for folders, a 20px
  file-type icon, and a star that sits at 0.35 opacity until the row is hovered
  or the file is bookmarked. Selection is Held Ink plus the 2px ember inset
  edge plus a brightened label.
- **Section headers:** 11px uppercase 700 at 0.06em in Faded Parchment, over
  Shadowed Ink, with a bottom hairline.

### Context Menu

Custom, with no dependency. Fixed-positioned at the pointer, 180px minimum,
Raised Ink, 10px radius, 1px border, menu lift shadow, 4px padding. Items are
12.5px with a 16px leading icon in Faded Parchment, 5px radius, and hover to
Held Ink. Destructive items carry Signal Coral text. Separators are a 1px Rule
Ink line inset 6px.

### File Notice

The shared shape for "this file is not readable right now": deleted,
unreadable, permission denied. Centered in the pane over the reading field: a
28px outline glyph, a title in the reader's language, the path in Path
Parchment mono, the raw reason as secondary text, and a row of actions capped
at two. Never a raw error kind as the headline.

### Toolbar

Three bands separated by 1px `.toolbar-sep` rules at 18px tall: navigation
(back, forward, reload), the address field, then custody (bookmark) and
hand-off (copy, share, Slack). The zoom chip lives inside the address field,
so showing it shrinks the input instead of shifting every button.

### Notice Toast

Transient messages about events outside the reading flow — a rejected deep
link — float bottom-left over the preview on Raised Ink with the menu lift
shadow, a 4px rise on entry, and an explicit dismiss. They never sit in the
layout column: a strip that appears and expires moves the artifact under the
reader's eyes.

### Quick Open

The signature retrieval surface. A `rgba(10, 8, 5, 0.4)` scrim, then a
`min(560px, 86vw)` panel at 12vh, capped at 60vh: search row, scrolling result
list, and a 10px footer hint. Each result is a single row of filename plus a
right-aligned dimmed monospace directory, so the eye reads names down the left
and paths down the right. The selected row takes Held Ink with no ember — the
list is a scan surface, and marking every keystroke with the accent would burn
it out.

### Start Page

The one expressive surface, shown on an empty tab. A 560px column rises 6px
into place over 360ms on mount. It leads with the New York wordmark, a
monospace workspace path beneath it, a row of ghost actions with keyboard
chips, then Bookmarks and Recents as labelled sections of icon rows.

## Do's and Don'ts

### Do:

- **Do** define every new color as a theme token in both the `:root` ink block
  and the `[data-theme="light"]` paper block. A hard-coded hex in a component
  breaks the OS-driven theme switch.
- **Do** mark active and selected state with a 2px ember bar on an edge
  (`::after` for tabs, `box-shadow: inset` for rows).
- **Do** keep the accent to markers: activity, selection, focus, held state,
  and links.
- **Do** render every filesystem path in `mono`, dimmed to Ghost Parchment and
  right-aligned when it trails a filename.
- **Do** hold the 4px spacing grid and the 4/6/10px radius scale.
- **Do** transition on `var(--dur-fast)` (80ms) for state feedback and
  `var(--dur)` (160ms) for larger moves, both on `var(--ease)`.
- **Do** give every interactive element a `:focus-visible` ember ring, and
  keep `prefers-reduced-motion` support intact.
- **Do** reveal icon-only affordances (close, star, remove) on hover of their
  row, on `:focus-visible`, and whenever the row is active. A focusable
  control at `opacity: 0` is a keyboard trap the eye cannot find.
- **Do** truncate paths from the head (`direction: rtl` plus a `<bdi>` child).
  The tail is the part that identifies the file.
- **Do** draw file-type icons with `currentColor` and let the row supply the
  tone.

### Don't:

- **Don't** introduce cool grey. No `#2a2a2a`, no blue-grey editor chrome, no
  neutral `rgba(0,0,0,x)` shadows.
- **Don't** add a second accent hue. Ember is the only chromatic voice; coral
  is failure, not color.
- **Don't** use `backdrop-filter`, frosted panels, or translucent chrome.
  Surfaces are opaque and warm.
- **Don't** exceed a 10px radius, and don't build pills, capsules, or circular
  containers.
- **Don't** put a resting shadow on a surface that sits inside the layout.
- **Don't** add a second serif element; the wordmark owns that voice.
- **Don't** let chrome grow with the window, and don't add responsive
  breakpoints. The sidebar split is the only elastic axis.
- **Don't** fill an element with the accent to say "selected". Use the 2px
  marker and a tone step.
- **Don't** give file-type icons per-language colors. Shape carries the type.
- **Don't** put a transient message in the layout column where its arrival and
  expiry move the artifact.
- **Don't** show a machine error kind as a headline. Name the problem, then
  offer the raw reason as detail.
