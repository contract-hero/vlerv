---
name: Vlervtifacts
description: A near-black artifact reading room in the Linear idiom — a four-step surface ladder, hairline borders instead of shadows, one lavender-blue accent spent on brand, focus, links and the current-object marker, and SF Pro display type at 500–600 with negative tracking. The chrome is a dark frame; the artifact is the protagonist.
colors:
  accent: "#5e6ad2"
  accent-strong: "#828fff"
  accent-focus: "#5e69d1"
  bg: "#010102"
  bg-chrome: "#0f1011"
  bg-row-hover: "#141516"
  bg-elevated: "#18191a"
  bg-row-selected: "#191a1b"
  fg: "#f7f8f8"
  heading-fg: "#f7f8f8"
  fg-strong: "#d0d6e0"
  fg-muted: "#8a8f98"
  fg-dim: "#62666d"
  fg-path: "#8a8f98"
  label-active-fg: "#ffffff"
  on-accent: "#ffffff"
  border: "#23252a"
  border-strong: "#34343a"
  border-tertiary: "#3e3e44"
  error-fg: "#eb5757"
  code-bg: "#141516"
  code-fg: "#d0d6e0"
  pre-bg: "#0f1011"
  button-hover-bg: "#23252a"
  badge-bg: "#18191a"
  badge-fg: "#d0d6e0"
  empty-fg: "#8a8f98"
  input-bg: "#0f1011"
  input-border: "#23252a"
  input-border-focus: "#34343a"
  resizer-hover: "#5e6ad2"
  star-active: "#5e6ad2"
  star-idle: "#3e3e44"
  scrollbar-thumb: "#2c2e33"
  scrollbar-thumb-hover: "#3e3e44"
  overlay-scrim: "rgba(0, 0, 0, 0.6)"
typography:
  display-md:
    fontFamily: "-apple-system, BlinkMacSystemFont, \"SF Pro Display\", Inter, system-ui, sans-serif"
    fontSize: "40px"
    fontWeight: 600
    lineHeight: 1.15
    letterSpacing: "-0.025em"
  headline:
    fontFamily: "-apple-system, BlinkMacSystemFont, \"SF Pro Display\", Inter, system-ui, sans-serif"
    fontSize: "2em"
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: "-0.03em"
  card-title:
    fontFamily: "-apple-system, BlinkMacSystemFont, \"SF Pro Display\", Inter, system-ui, sans-serif"
    fontSize: "1.5em"
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: "-0.02em"
  body:
    fontFamily: "-apple-system, BlinkMacSystemFont, \"SF Pro Text\", Inter, system-ui, sans-serif"
    fontSize: "16px"
    fontWeight: 400
    lineHeight: 1.5
    letterSpacing: "-0.003em"
  body-sm:
    fontFamily: "-apple-system, BlinkMacSystemFont, \"SF Pro Text\", Inter, system-ui, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.5
    letterSpacing: "-0.003em"
  ui:
    fontFamily: "-apple-system, BlinkMacSystemFont, \"SF Pro Text\", Inter, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.4
    letterSpacing: "-0.003em"
  caption:
    fontFamily: "-apple-system, BlinkMacSystemFont, \"SF Pro Text\", Inter, system-ui, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.4
    letterSpacing: "-0.003em"
  eyebrow:
    fontFamily: "-apple-system, BlinkMacSystemFont, \"SF Pro Text\", Inter, system-ui, sans-serif"
    fontSize: "12px"
    fontWeight: 500
    lineHeight: 1.3
    letterSpacing: "0.4px"
  button:
    fontFamily: "-apple-system, BlinkMacSystemFont, \"SF Pro Text\", Inter, system-ui, sans-serif"
    fontSize: "14px"
    fontWeight: 500
    lineHeight: 1.2
    letterSpacing: "0"
  mono:
    fontFamily: "\"SF Mono\", ui-monospace, SFMono-Regular, \"JetBrains Mono\", Menlo, monospace"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.5
    letterSpacing: "0"
rounded:
  xs: "4px"
  sm: "6px"
  md: "8px"
  lg: "12px"
  pill: "9999px"
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
    typography: "{typography.ui}"
    padding: "0 8px 0 12px"
    height: "38px"
  tab-active:
    backgroundColor: "{colors.bg}"
    textColor: "{colors.fg}"
    typography: "{typography.ui}"
    padding: "0 8px 0 12px"
    height: "38px"
  toolbar-button:
    backgroundColor: "transparent"
    textColor: "{colors.fg-muted}"
    rounded: "{rounded.sm}"
    height: "28px"
    width: "28px"
  toolbar-button-hover:
    backgroundColor: "{colors.button-hover-bg}"
    textColor: "{colors.fg}"
    rounded: "{rounded.sm}"
    height: "28px"
    width: "28px"
  address-input:
    backgroundColor: "{colors.input-bg}"
    textColor: "{colors.fg}"
    typography: "{typography.mono}"
    rounded: "{rounded.md}"
    padding: "5px 12px"
  status-badge:
    backgroundColor: "{colors.badge-bg}"
    textColor: "{colors.badge-fg}"
    typography: "{typography.caption}"
    rounded: "{rounded.pill}"
    padding: "2px 8px"
  explorer-row:
    backgroundColor: "transparent"
    textColor: "{colors.fg-strong}"
    typography: "{typography.body-sm}"
    padding: "0 12px 0 0"
    height: "30px"
  explorer-row-selected:
    backgroundColor: "{colors.bg-row-selected}"
    textColor: "{colors.label-active-fg}"
    typography: "{typography.body-sm}"
    padding: "0 12px 0 0"
    height: "30px"
  button-primary:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.on-accent}"
    typography: "{typography.button}"
    rounded: "{rounded.md}"
    padding: "8px 14px"
  button-primary-hover:
    backgroundColor: "{colors.accent-strong}"
    textColor: "{colors.on-accent}"
    typography: "{typography.button}"
    rounded: "{rounded.md}"
    padding: "8px 14px"
  button-secondary:
    backgroundColor: "{colors.bg-chrome}"
    textColor: "{colors.fg}"
    typography: "{typography.button}"
    rounded: "{rounded.md}"
    padding: "8px 14px"
  brand-mark:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.on-accent}"
    typography: "{typography.body}"
    rounded: "{rounded.md}"
    height: "34px"
    width: "34px"
  eyebrow:
    backgroundColor: "transparent"
    textColor: "{colors.fg-muted}"
    typography: "{typography.eyebrow}"
  quick-open:
    backgroundColor: "{colors.bg-elevated}"
    textColor: "{colors.fg}"
    typography: "{typography.body}"
    rounded: "{rounded.lg}"
    padding: "0"
  context-menu:
    backgroundColor: "{colors.bg-elevated}"
    textColor: "{colors.fg-strong}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.lg}"
    padding: "4px"
  context-menu-item:
    backgroundColor: "transparent"
    textColor: "{colors.fg-strong}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.sm}"
    padding: "6px 10px"
  app-notice:
    backgroundColor: "{colors.bg-elevated}"
    textColor: "{colors.error-fg}"
    typography: "{typography.ui}"
    rounded: "{rounded.lg}"
    padding: "8px 8px 8px 12px"
  code-block:
    backgroundColor: "{colors.pre-bg}"
    textColor: "{colors.fg-strong}"
    typography: "{typography.mono}"
    rounded: "{rounded.md}"
    padding: "14px 16px"
  kbd-chip:
    backgroundColor: "{colors.badge-bg}"
    textColor: "{colors.fg-muted}"
    typography: "{typography.caption}"
    rounded: "{rounded.xs}"
    padding: "1px 5px"
---

## Overview

Vlervtifacts is a macOS reading room for local HTML and Markdown artifacts.
Its design language is Linear's: a near-black canvas, a four-step surface
ladder, 1px hairline borders in place of shadows, and one lavender-blue accent
spent scarcely.

The system's whole argument is that **the chrome is a dark frame and the
artifact is the protagonist**. Linear's marketing pages make that argument with
product screenshots framed in charcoal panels. This app makes it literally: an
artifact rendered in the iframe IS the screenshot, and the chrome around it is
the panel.

**Plane order, deepest first.** The reading field (`{colors.bg}`) sits at canvas
depth and the chrome (`{colors.bg-chrome}`) lifts one step above it. This
inverts the previous system on purpose. Putting the artifact on the deepest
surface means a self-contained HTML page — which almost always paints its own
background — reads as a lifted panel inside a dark frame, exactly the
relationship Linear builds between a page and a product screenshot.

**Key characteristics:**

- **Near-black canvas.** `{colors.bg}` is #010102, not `#000000`. The faint
  blue tint is intentional and is what keeps the surface from reading as a hole.
- **Four-step surface ladder** carries every hierarchy: canvas → chrome →
  row-hover → elevated → row-selected. No level is skipped.
- **Hairlines, not shadows.** Every boundary in the layout is a 1px border.
  Shadow survives only on floating overlays, which have no surface to lift
  against.
- **One lavender accent**, `{colors.accent}` #5e6ad2, with exactly four jobs.
- **Negative tracking on display, positive on the eyebrow.** The reversal is
  what marks a label as taxonomy rather than voice.
- **Two themes.** Linear ships no light marketing surface; this app must,
  because macOS system appearance is a design surface (PRODUCT.md).

## The One Lavender Rule

`{colors.accent}` does four jobs and no others:

1. **Brand mark** — the monogram tile on the start page.
2. **Focus ring** — every `:focus-visible` outline.
3. **Link emphasis** — Markdown links, in the lighter step.
4. **Current-object marker** — the 2px rule on the active tab, the 2px inset
   edge on the selected explorer row and the selected Quick Open row, and drop
   targets during a drag.

Two in-product roles extend job 4 rather than adding a fifth: the bookmark star
(`{colors.star-active}`) and the sidebar resizer on hover
(`{colors.resizer-hover}`) both mark an object the user has singled out.

Everything else — a quotation rule, a separator, a hover state, a badge, a
count, a file glyph — is ink or hairline. A blockquote edge takes
`{colors.border-strong}`, not the accent.

### Accent steps

- `{colors.accent}` #5e6ad2 — structural markers and filled CTAs. Holds 4.4:1
  on canvas, which clears the 3:1 a non-text marker needs.
- `{colors.accent-strong}` #828fff — hover on a filled CTA, and **all body-text
  links**. #5e6ad2 lands at 4.4:1 on canvas, just under the 4.5:1 text has to
  hold; #828fff clears 7.2:1.
- `{colors.accent-focus}` #5e69d1 — the focus ring and the pressed CTA.

## Colors

### Surface ladder

| Token | Value | Role |
|---|---|---|
| `{colors.bg}` | #010102 | Reading field, active tab, iframe backdrop |
| `{colors.bg-chrome}` | #0f1011 | Sidebar, toolbar, tab strip, table headers, code blocks |
| `{colors.bg-row-hover}` | #141516 | Row hover, inline code |
| `{colors.bg-elevated}` | #18191a | Quick Open, context menu, toast, badges |
| `{colors.bg-row-selected}` | #191a1b | Selected row, selected menu item |

### Hairlines

| Token | Value | Role |
|---|---|---|
| `{colors.border}` | #23252a | Every boundary in the layout — pane seams, band rules, table cells, code-block edges |
| `{colors.border-strong}` | #34343a | Floating-overlay edges, hovered button edges, blockquote rule, focused input |
| `{colors.border-tertiary}` | #3e3e44 | Nested surfaces, idle star, scrollbar hover |

### Ink ladder

| Token | Value | Role | Floor |
|---|---|---|---|
| `{colors.fg}` | #f7f8f8 | Headings, active labels, primary chrome text | — |
| `{colors.fg-strong}` | #d0d6e0 | Reading-field body, explorer file labels, menu items | — |
| `{colors.fg-muted}` | #8a8f98 | Secondary chrome, eyebrows, captions, hints | 5.4:1 on the lightest surface |
| `{colors.fg-dim}` | #62666d | Decoration and disabled only — chevrons, idle stars, disabled buttons | 3.1:1 — never carries text a user must read |
| `{colors.fg-path}` | #8a8f98 | Directory paths | 4.5:1 everywhere, by definition |

`{colors.fg-path}` holds the same value as `{colors.fg-muted}`. It stays a
separate token because paths are the information that tells two same-named
artifacts apart, so their contrast floor is a constraint worth naming and
checking. In the previous palette that constraint forced a distinct value; on
this ladder `{colors.fg-muted}` already satisfies it.

`{colors.fg-dim}` is the one token that does not clear 4.5:1. It is allowed on
decoration and disabled states only. Informational micro-text — the Quick Open
footer, the bookmarks empty hint, the start-page subtitle, section eyebrows —
uses `{colors.fg-muted}`.

### Semantic

- Linear's one documented semantic, success green (#27a644), has **no role in
  this product yet**, so it is recorded here and not declared in the stylesheet.
- `{colors.error-fg}` #eb5757 — Linear's source lists error styling as a known
  gap. This is Linear's in-product red; it clears 5.9:1 on canvas. It is not a
  second accent: it appears only on failure text (unreadable file, rejected
  deep link, destructive menu item, bookmark removal on hover).

### Light theme

Derived from Linear's documented inverse tokens (`inverse-canvas`,
`inverse-surface-1/2`, `inverse-ink`). The plane relationship is preserved: the
reading field is the extreme (`#ffffff`), the chrome one step in (`#f0f1f3`).

The source's `inverse-surface-1` is `#f5f6f6`, only 2% off white. Across a full
window that reads as one flat field rather than as two planes, so the chrome and
the hairlines each take one extra step: chrome `#f0f1f3`, hairline `#dcdee3`.
This is a documented deviation, taken because the dark theme's plane separation
has to survive the theme switch.

| Token | Dark | Light |
|---|---|---|
| `{colors.bg}` | #010102 | #ffffff |
| `{colors.bg-chrome}` | #0f1011 | #f0f1f3 |
| `{colors.bg-row-hover}` | #141516 | #e5e7eb |
| `{colors.bg-row-selected}` | #191a1b | #dcdfe6 |
| `{colors.border}` | #23252a | #dcdee3 |
| `{colors.border-strong}` | #34343a | #c5c8d0 |

The accent does not change hue across themes — #5e6ad2 holds 4.7:1 on white.
Only the emphasis step darkens, to `#4a55b8`.

## Typography

### Families

| Token | Stack | Role |
|---|---|---|
| display | `-apple-system, BlinkMacSystemFont, "SF Pro Display", Inter, system-ui` | Wordmark, brand mark, Markdown headings, file-notice title |
| ui | `-apple-system, BlinkMacSystemFont, "SF Pro Text", Inter, system-ui` | Everything else |
| mono | `"SF Mono", ui-monospace, SFMono-Regular, "JetBrains Mono", Menlo` | Paths, address bar, code, kbd chips, metadata |

Linear Display and Linear Text are proprietary. On macOS the documented
substitute is SF Pro, which `-apple-system` resolves to exactly; Inter is the
cross-platform fallback the source names. Display and Text are treated as one
continuous voice — the family change is silent.

There is **no serif anywhere**. The previous system spent New York on the
wordmark; this one carries the wordmark with the display cut at weight 600 and
tracking pulled to -0.025em. Weight and tracking are the whole gesture.

### Scale

Linear's published scale is a marketing scale. The chrome of a dense desktop app
lives one step below its body size, so the ramp is anchored differently: the
**reading field** takes Linear's `body`, and the **chrome** takes a 13px step
between `caption` and `body-sm`.

| Token | Size | Weight | Tracking | Use |
|---|---|---|---|---|
| `{typography.display-md}` | 40px | 600 | -0.025em | The wordmark. One instance in the product. |
| `{typography.headline}` | 2em | 600 | -0.03em | Markdown `h1` |
| `{typography.card-title}` | 1.5em | 600 | -0.02em | Markdown `h2` |
| `{typography.body}` | 16px | 400 | -0.003em | Reading field, Quick Open input, file-notice title |
| `{typography.body-sm}` | 14px | 400 | -0.003em | Explorer rows, start rows, menu items, bookmarks, tables |
| `{typography.ui}` | 13px | 400 | -0.003em | Chrome default — tabs, sidebar header, toolbar, address bar |
| `{typography.caption}` | 12px | 400 | -0.003em | Paths, badges, hints, footers, kbd chips |
| `{typography.eyebrow}` | 12px | 500 | **+0.4px** | Section labels |
| `{typography.button}` | 14px | 500 | 0 | All button labels |
| `{typography.mono}` | 13px | 400 | 0 | Code, plain text, metadata |

### The eyebrow reversal

Every other step in the ramp tracks negative. The eyebrow tracks **positive**,
at +0.4px, weight 500, sentence case. That reversal is the whole signal — it is
what marks "Bookmarks" or "Recent" as taxonomy rather than voice.

It replaces the uppercase 10–11px/700 micro-caps of the previous system. Do not
reintroduce `text-transform: uppercase`; the tracking carries the job.

## Layout

### Spacing

Base unit 4px. Tokens: 4 · 8 · 12 · 16 · 24 · 32.

### Fixed bands

The window is a column: one full-width tab strip, then a row of two panes. Band
heights are structural and do not scale:

- **38px** — the tab strip. It spans the **whole window**, above both panes, and
  doubles as the overlay title bar: it carries `data-tauri-drag-region` and
  reserves a **78px gutter** for the macOS traffic lights before the first tab.
  It used to sit inside the preview pane, which left a dead 360px band above the
  sidebar whose only occupant was the traffic lights.
- **40px** — the sidebar header and the toolbar. Equal by requirement: they sit
  side by side under the tab strip, so their bottom borders must form one
  unbroken rule rather than a 1px step at the sidebar seam.
- **30px** — an explorer row.

### Panes

The sidebar is the only elastic axis: 200px minimum, 480px maximum. The reading
field takes the rest.

The resizer is an **overlay, not a column**. It takes zero width in the layout
and carries its 6px pointer target in a pseudo-element straddling the seam,
painting `{colors.resizer-hover}` at 60% opacity while hovered or dragging. As a
6px flex item it opened a transparent gap that ran the full height of the
window — including straight through the chrome bands, where it broke the very
rule those band heights exist to produce.

There are no responsive breakpoints. This is a macOS window on one machine, not
a page.

### Reading measure

The start page holds a 560px column. The reading field imposes no measure — an
artifact controls its own layout, and clamping it would break the fidelity the
product exists to deliver.

## Elevation & Depth

| Level | Treatment | Use |
|---|---|---|
| 0 | No border, no shadow | Reading field, body text |
| 1 | `{colors.bg-chrome}` + 1px `{colors.border}` | Sidebar, toolbar, tab strip, code blocks |
| 2 | `{colors.bg-row-hover}` | Hovered rows |
| 3 | `{colors.bg-elevated}` + 1px `{colors.border-strong}` + shadow + edge highlight | Quick Open, context menu, toast |
| 4 | 2px `{colors.accent-focus}` outline, -2px offset | Focus |

### The edge highlight

Lifted panels carry `inset 0 1px 0 rgba(255, 255, 255, 0.06)` on their top
edge. It is a single hairline of white light and it is the system's signature
detail — the thing that makes a dark panel read as rendered rather than as an
absence. Apply it to floating overlays only.

### On shadows

Linear resists drop shadows on dark almost entirely, and so does this system.
The three floating overlays keep one because they have no surface below them to
lift against. Their shadow is **pure black** — a tinted shadow would introduce a
hue the palette does not contain.

Nothing that sits inside the layout gets a resting shadow.

### On the focus ring

Linear specifies a 2px `primary-focus` ring at 50% opacity. At 50% over a
#010102 canvas that lands near 1.9:1 against the surface it marks — below the
3:1 a focus indicator has to hold. **The ring ships solid.** The size, color and
role are the source's; the alpha is a documented deviation, taken for
accessibility.

## Shapes

| Token | Value | Use |
|---|---|---|
| `{rounded.xs}` | 4px | kbd chips, tab close, star toggle, inline code, focus-ring radius |
| `{rounded.sm}` | 6px | Toolbar buttons, menu items, list rows, Quick Open rows |
| `{rounded.md}` | 8px | All buttons, all inputs, code blocks, the brand mark |
| `{rounded.lg}` | 12px | Floating overlays — Quick Open, context menu, toast |
| `{rounded.pill}` | 9999px | Status badges, the zoom control, the loading dot, scrollbar thumbs |

Pills are for **status**, never for actions. A CTA is 8px, always — Linear's
"don't pill-round CTAs" holds here.

## Components

### Tabs

The active tab drops to `{colors.bg}` — canvas depth — and carries the 2px
lavender rule on its top edge. Because the strip now spans the window, an active
tab sitting over the sidebar no longer joins the reading field below it; the
canvas fill and the lavender rule carry the state on their own, and the tab
reads as lifted rather than as continuous.

An inactive tab is transparent over `{colors.bg-chrome}` and lifts to
`{colors.bg-row-hover}` on hover. A tab loading its file shows a 6px lavender
dot pulsing between 25% and 100% opacity.

The close button sits at `opacity: 0` until the tab is hovered, active, or the
button itself holds focus. A focusable control must be visible when focused —
an invisible button in the tab order is a target the keyboard reaches and the
eye cannot find.

### Explorer rows

30px, 14px type, `{colors.fg-strong}`. Folders take `{colors.fg}` at weight 500;
files take `{colors.fg-strong}` at 400 and step to `{colors.label-active-fg}` on
hover.

Selection is **a surface lift plus the current-object marker** — a single step
on this ladder cannot carry the state alone, so `{colors.bg-row-selected}` pairs
with a 2px lavender inset edge. Never fill a row with the accent.

File glyphs carry shape, never hue. Two tones only: `is-subject` marks the
artifacts this app exists to read (`.html`, `.md`) and sits one ink step
brighter; everything else recedes. Both step up together on hover so the glyph
follows its row instead of competing with it.

### Buttons

`{components.button-primary}` is the one filled button in the product — the
first-run workspace picker and the actions on a file notice. Lavender fill,
white label, 8px corners, 8px/14px padding, label at 14px weight 500. Hover
lifts to `{colors.accent-strong}`; pressed drops to `{colors.accent-focus}`.

`{components.button-secondary}` is charcoal: `{colors.bg-chrome}` fill,
`{colors.border}` hairline, ink label. Its hairline strengthens on hover.

### Address bar

Mono type on `{colors.input-bg}`, 8px corners, hairline border that strengthens
to `{colors.input-border-focus}` on focus. A path is machine text; it is set in
mono everywhere it appears — address bar, explorer path column, Quick Open
directory, metadata renderer, file-notice path.

### Floating overlays

Quick Open, the context menu and the toast share one shape: `{colors.bg-elevated}`
fill, 12px corners, `{colors.border-strong}` hairline, shadow, edge highlight.
The scrim behind Quick Open is pure black at 60%.

Quick Open truncates paths at the **tail**; the start page truncates at the
**head**. The difference is not decoration. Quick Open paths are relative to the
workspace root, so the leading segment is the repo name — the one part that
disambiguates two files with the same name. Start-page recents can be absolute
paths sharing a long `/Users/…/workspace/` prefix, so there the head is the part
worth dropping.

### Start page

A brand mark and a wordmark, then actions, then bookmarks and recents. The mark
is a 34px lavender tile at 8px corners carrying the monogram in the display cut.
It is the only filled lavender surface in the product besides the primary button.

### Markdown

The reading field runs `{typography.body}`: 16px, line-height 1.5. Headings step
to the display cut at weight 600 with tracking pulling negative as size grows.
`h1` and `h2` keep a hairline bottom rule — the same gesture Linear's changelog
rows use.

## Do's and Don'ts

### Do

- Keep `{colors.bg}` at #010102. The faint blue tint is the point.
- Move one step at a time on the surface ladder.
- Draw every in-layout boundary as a 1px hairline.
- Spend lavender on the four jobs in The One Lavender Rule, and count them.
- Pull tracking negative on display, positive on the eyebrow.
- Pair a surface lift with the 2px marker to say "current".
- Give floating overlays the edge highlight.
- Set paths in mono, everywhere.
- Keep `{colors.fg-dim}` off anything a user has to read.

### Don't

- **Don't** use `#000000` as the canvas.
- **Don't** introduce a second chromatic accent. Red is failure, not color;
  green is the one documented semantic and appears nowhere yet.
- **Don't** use lavender as a section background, a row fill, or a hover state.
- **Don't** fill an element with the accent to say "selected". Use the 2px
  marker and a tone step.
- **Don't** put a resting shadow on a surface that sits inside the layout.
- **Don't** tint a shadow. Pure black only.
- **Don't** reintroduce a serif, or any second display family.
- **Don't** reintroduce uppercase micro-caps. The eyebrow's positive tracking
  is the label signal.
- **Don't** pill-round an action. Pills mark status.
- **Don't** add atmospheric gradients, spotlight cards, `backdrop-filter`, or
  translucent chrome. Surfaces are opaque.
- **Don't** give file-type glyphs per-language colors. Shape carries the type.
- **Don't** let chrome grow with the window, and don't add breakpoints. The
  sidebar split is the only elastic axis.
- **Don't** put a transient message in the layout column, where its arrival and
  expiry move the artifact under the reader's eyes.
- **Don't** show a machine error kind as a headline. Name the problem, then
  offer the raw reason as detail.

## Known Gaps

- Success green is recorded above but not declared. Nothing in the product
  reports success as a state yet.
- Framing the iframe in a 16px panel would complete Linear's product-screenshot
  idiom, but it costs reading width, so the scale stops at 12px.
- `src/components/Settings.tsx` is not mounted by any route and carries no
  styles. It is outside this system until it ships.
- Shiki keeps `github-dark` / `github-light` for syntax highlighting. Its
  backgrounds are overridden to transparent so the block takes
  `{colors.pre-bg}`, but the token hues are GitHub's, not Linear's.
