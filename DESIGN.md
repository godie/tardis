---
name: TARDIS v1
description: Real-time transcription and translation desktop shell
colors:
  paper: "oklch(0.96 0.02 78)"
  paper-shadow: "oklch(0.9 0.02 78)"
  lift: "oklch(1 0 78)"
  ink: "oklch(0.24 0.03 38)"
  ink-soft: "oklch(0.42 0.03 42)"
  ochre: "oklch(0.69 0.16 56)"
  ochre-deep: "oklch(0.57 0.13 48)"
  ochre-faint: "oklch(0.91 0.04 65)"
  ochre-surface: "oklch(0.94 0.045 62)"
  action-fill: "oklch(0.53 0.14 52)"
  action-fill-deep: "oklch(0.48 0.13 45)"
  action-label: "oklch(0.97 0.015 80)"
  success: "oklch(0.47 0.1 152)"
  success-surface: "oklch(0.95 0.04 152)"
  danger: "oklch(0.44 0.17 28)"
  danger-surface: "oklch(0.95 0.035 28)"
  danger-strong: "oklch(0.62 0.19 28)"
  paper-dark: "oklch(0.18 0.02 78)"
  paper-shadow-dark: "oklch(0.12 0.02 78)"
  lift-dark: "oklch(0.26 0.015 78)"
  ink-dark: "oklch(0.88 0.02 78)"
  ink-soft-dark: "oklch(0.68 0.02 78)"
  ochre-dark: "oklch(0.72 0.14 62)"
  ochre-deep-dark: "oklch(0.62 0.13 52)"
  ochre-faint-dark: "oklch(0.28 0.04 62)"
  ochre-surface-dark: "oklch(0.3 0.05 62)"
  success-dark: "oklch(0.82 0.1 152)"
  success-surface-dark: "oklch(0.28 0.045 152)"
  danger-dark: "oklch(0.85 0.08 28)"
  danger-surface-dark: "oklch(0.29 0.06 28)"
  danger-strong-dark: "oklch(0.78 0.14 28)"
typography:
  display:
    fontFamily: "Iowan Old Style, Palatino Linotype, Book Antiqua, Georgia, serif"
    fontSize: "clamp(3rem, 7vw, 5.8rem)"
    fontWeight: 700
    lineHeight: 0.95
    letterSpacing: "0em"
  headline:
    fontFamily: "Iowan Old Style, Palatino Linotype, Book Antiqua, Georgia, serif"
    fontSize: "clamp(1.8rem, 3vw, 2.4rem)"
    fontWeight: 600
    lineHeight: 1
    letterSpacing: "0em"
  title:
    fontFamily: "Iowan Old Style, Palatino Linotype, Book Antiqua, Georgia, serif"
    fontSize: "clamp(1.6rem, 2.6vw, 2.1rem)"
    fontWeight: 600
    lineHeight: 1
    letterSpacing: "0em"
  body:
    fontFamily: "Avenir Next, Segoe UI, Helvetica Neue, sans-serif"
    fontSize: "1rem"
    fontWeight: 400
    lineHeight: 1.8
    letterSpacing: "0em"
  label:
    fontFamily: "Avenir Next, Segoe UI, Helvetica Neue, sans-serif"
    fontSize: "0.72rem"
    fontWeight: 400
    lineHeight: 1.5
    letterSpacing: "0.2em"
rounded:
  pill: "999px"
  panel: "32px"
  strip: "28px"
  field: "24px"
  nested: "22px"
  input: "16px"
  notice: "14px"
  status: "12px"
spacing:
  xs: "8px"
  sm: "12px"
  md: "18px"
  lg: "24px"
  xl: "32px"
components:
  button-primary:
    backgroundColor: "{colors.action-fill}"
    textColor: "{colors.action-label}"
    rounded: "{rounded.pill}"
    padding: "14px 22px"
    typography: "{typography.body}"
  button-secondary:
    backgroundColor: "{colors.lift}"
    textColor: "{colors.ink}"
    rounded: "{rounded.pill}"
    padding: "14px 22px"
    typography: "{typography.body}"
  card:
    backgroundColor: "{colors.lift}"
    textColor: "{colors.ink}"
    rounded: "{rounded.panel}"
    padding: "24px"
  status-pill:
    backgroundColor: "{colors.ochre-faint}"
    textColor: "{colors.ink}"
    rounded: "{rounded.pill}"
    padding: "10px 16px"
    typography: "{typography.label}"
  field-select:
    backgroundColor: "{colors.lift}"
    textColor: "{colors.ink}"
    rounded: "{rounded.input}"
    padding: "14px 40px 14px 16px"
  status-done:
    backgroundColor: "{colors.success-surface}"
    textColor: "{colors.ink}"
    rounded: "{rounded.status}"
    padding: "10px 14px"
  status-error:
    backgroundColor: "{colors.danger-surface}"
    textColor: "{colors.danger}"
    rounded: "{rounded.status}"
    padding: "10px 14px"
---

# Design System: TARDIS v1

## Overview

**Creative North Star: "The Signal Desk"**

TARDIS v1 is a desk you sit at to listen. Warm paper is the surface, ink is the hand, and ochre is the signal being tuned in. The interface is not a dashboard that reports on audio; it is an instrument you operate, and its color vocabulary exists to tell you what the instrument is doing right now. Idle is quiet, listening is lit, and a failure is unmistakably a failure — not a shade of idle.

The material character is handmade rather than clinical. A book serif carries every heading, the paper gradient runs warm to warm, and the only truly dark tone on the screen belongs to the primary action. That single reserved darkness is the whole hierarchy: the eye lands on the seal, not on the furniture. Cards are soft and slightly raised, with generous corners and one shared ambient shadow, so they read as sheets laid on the desk rather than as regions cut out of a screen.

Confirmed anti-reference: this must never drift into the dark SaaS dashboard — near-black canvas, cyan or violet accent, glassmorphism, hero metrics. The warmth and the serif are the product's identity; losing them for a "cleaner" admin look is the failure mode.

**Key Characteristics:**
- Warm paper ground with a serif display voice and an ink-soft sans body.
- Ochre is the only chroma on the page until a state needs to speak.
- Color encodes state: idle, starting, listening, stopped, done, failed each own a fill.
- One ambient shadow for every surface; depth is constant, not tiered.
- The primary action holds the deepest tone in the palette.

## Colors

A two-temperature palette: warm paper and dark ink, bridged by a single ochre family, with red and green admitted only as state.

### Primary
- **Deep Ochre Action** (`oklch(0.53 0.14 52)` → `oklch(0.48 0.13 45)` in a 135° gradient): the primary and file-transcribe buttons, and nothing else. It is deliberately darker than every other ochre on the page, because a mid-tone ochre cannot hold a label at AA in either direction — it is too light for white text and too dark for ink. Deepening the fill is what lets the label stay paper-white at 5.07:1 in both themes.
- **Paper Label** (`oklch(0.97 0.015 80)`): text on the action fill, theme-invariant. Never white; never the ink token, which flips to light in dark mode.

### Secondary
- **Signal Ochre** (`oklch(0.69 0.16 56)`): the live state — the listening dot and its halo, the focus ring, the busy fill, the faint grid on the paper. If it is ochre and full strength, the instrument is working.
- **Deep Signal** (`oklch(0.57 0.13 48)`): the dot and left bars that must stay legible at 10px, plus the focus ring in light mode.
- **Faint Signal** (`oklch(0.91 0.04 65)`): the resting tint behind an idle status pill and the warm body of every select.
- **Signal Surface** (`oklch(0.94 0.045 62)`): the fill for "armed but not live" — the starting state and the busy file status.

### Tertiary
- **Reading Green** (`oklch(0.47 0.1 152)` on `oklch(0.95 0.04 152)`): completed work only — a finished WAV transcription. Never used for "running well"; that is the ochre's job.
- **Seal Red** (`oklch(0.44 0.17 28)` on `oklch(0.95 0.035 28)`, with `oklch(0.62 0.19 28)` as the bar/dot): every failure, in both the live error banner, the file error banner, and the failed status. One role, one set of values; three slightly different reds written by hand was the state before.

### Neutral
- **Paper** (`oklch(0.96 0.02 78)`) and **Paper Shadow** (`oklch(0.9 0.02 78)`): the two ends of the 135° page gradient. The page is never a flat fill.
- **Lift** (`oklch(1 0 78)`): everything that needs to read as above the paper — card fills, input fills, the top of the accent gradient. It is white carrying the paper hue, and that hue is load-bearing: a hue-less white mixes toward red in oklch and turns every tinted surface pink.
- **Ink** (`oklch(0.24 0.03 38)`) and **Ink Soft** (`oklch(0.42 0.03 42)`): primary and secondary text. Rarely gray — both carry a brown cast so type stays in the paper world.

Dark mode remaps the same roles instead of inverting them: paper-dark and paper-shadow-dark for the page, lift-dark above it, ink-dark and ink-soft-dark for type, and each ochre and semantic role gets a lighter, higher-chroma value so it reads against a dark ground. The two action tokens and the action label do not change, because the fill is deep enough in either theme.

### Named Rules
**The Rarity Rule.** Ochre earns attention because it is scarce. One lit state at a time, one action per view; never tint three surfaces in the same viewport "for warmth."

**The Lift Rule.** Surfaces mix toward `--lift`, never toward literal `white`. A `white` mix partner is how a dark theme silently turns into white text on white cards.

**The State Rule.** Status is never carried by color alone: every state also changes its wording or gains a bar/dot shape, and every state pair clears 4.5:1 against its own fill.

## Typography

**Display Font:** Iowan Old Style (with Palatino Linotype, Book Antiqua, Georgia)
**Body Font:** Avenir Next (with Segoe UI, Helvetica Neue)

**Character:** A printed-book serif against a humanist sans — the pairing of a notebook's headings and its margin notes. The serif is what makes a prototype shell feel authored instead of generated; the sans keeps dense control labels plain and scannable.

### Hierarchy
- **Display** (700, `clamp(3rem, 7vw, 5.8rem)`, 0.95): the product name only, capped at 8ch so it breaks as a stamp rather than a banner.
- **Headline** (600, `clamp(1.8rem, 3vw, 2.4rem)`, 1): panel titles — "Captured speech", "Rendered output".
- **Title** (600, `clamp(1.6rem, 2.6vw, 2.1rem)`, 1): the file-transcription panel heading, one step under headline so it stays subordinate to the live panels.
- **Body** (400, 1rem, 1.8): transcript and translation copy. Line-height 1.8 because transcript panels are read as running text, not as UI labels.
- **Label** (400, 0.72rem, 0.2em tracking, uppercase, ink-soft): field labels, panel kickers, the status label. Letter-spacing is generous enough that the caps read as a printed rubric.

### Named Rules
**The Two-Voice Rule.** Serif speaks in headings, sans speaks in controls. A serif on a button or a sans on a panel title breaks the desk metaphor.

## Layout

A single centered column, `min(1100px, 100vw - 32px)`, with 32–56px of top air and 48px below. Vertical rhythm is a fixed set of gaps rather than a scale-by-density system: 18px between cards in a grid, 12px inside a button row or the file-path row, 16px between the provider field and the action group.

The page reads top-down in the order a session happens: identity (hero) → instrument (status strip) → settings (2×2 field grid) → output (2×2 panel grid) → file tooling. The field grid and the panel grid are both two equal columns; below 880px every grid collapses to one column and the button row goes left-aligned. The shell ships in a Tauri window with a 960×680 minimum, so the collapsed layout is the floor, not a mobile target — the responsive rules exist to survive a narrow window, not to serve phones.

Cards carry 18–24px of internal padding (`control-strip` 22px, `panel` and `file-transcribe` 24px, `language-field` and nested panel 18px). Nothing is padded below 18px except status chips and notices.

## Elevation & Depth

Ambient, not tiered. Every card — control strip, language field, panel, file panel — sits under the same shadow, so all surfaces are floating at one height above the paper. Depth communicates "object on a desk", never "more important than the card next to it". There is no modal or elevated tier; nothing in this UI stacks over anything else.

### Shadow Vocabulary
- **Ambient Sheet** (`box-shadow: 0 24px 80px color-mix(in oklch, var(--ink) 18%, transparent)`): the only card shadow. In dark theme it becomes `black 40%`, since ink itself is light there.
- **Live Halo** (`box-shadow: 0 0 0 8px color-mix(in oklch, var(--signal) 18%, transparent)`): reserved for the listening dot. It is a state light, not an elevation.

### Named Rules
**The One Height Rule.** Do not introduce a second card shadow to signal hierarchy. If two cards must differ, they differ by content or by a state fill, not by elevation.

## Shapes

Generously rounded, never geometric. Cards sit at 28–32px, fields at 24px, inputs and selects at 16px, and every action and status chip is a full pill. The radius decreases as the element gets smaller and sits deeper: the page-level panels are the roundest, the nested transcript card is 22px, a notice is 14px, a status strip 12px. Borders are 1px at 12–14% ink — present enough to cut the card out of the warm background, never a drawn line.

Only one element is fully filled and borderless: the primary action. Its pill silhouette against the bordered cards is the shape signature of the system.

## Components

### Buttons
- **Shape:** full pill (999px), 14px × 22px padding, body sans at 700.
- **Primary:** the deep ochre gradient with a paper-white label. Hover and focus-visible lift it 1px; transitions run 160ms ease on transform, opacity, and background. Disabled drops to 0.6 opacity with `not-allowed`; busy drops to 0.7 with a progress cursor.
- **Secondary:** ink at 6% over lift, ink text, same pill. It is deliberately colorless — the action owns the warmth, and "Stop" must not compete with "Start Listening".

### Chips
- **Status pill:** the instrument readout. Idle is an ink-soft dot on a faint ochre bed; starting is a deep-ochre dot on the signal surface; listening is a deep-ochre dot inside an 8px ochre halo on a 38% ochre bed; stopped is a solid ink dot on a 9% ink bed; error is a seal-red dot on the danger surface with red text.
- **Status strips (file transcription):** a 3px left bar plus a tinted fill — neutral for idle, signal surface for busy, reading green for done, danger surface for failed. The bar is what carries the state for anyone who cannot separate the fills.

### Cards / Containers
- **Corner Style:** 32px for page-level cards, 22px for the card nested inside one.
- **Background:** lift, except the accent panel and file panel, which run a 155° gradient from a faint ochre-tinted lift to plain lift.
- **Shadow Strategy:** see Elevation — one ambient shadow for all.
- **Border:** 1px ink at 12% (light) / 14% (dark).
- **Internal Padding:** 24px, 18px when nested.

### Inputs / Fields
- **Style:** 16px radius, lift tinted 85% toward the faint signal, 1px ink border, 14–16px padding, ink text at full strength.
- **Focus:** a 2px outline in the focus-ring token at 2px offset. One ring for every control; a second, slightly different ring on one input is what this system had before.
- **Error / Disabled:** errors never live on the input. They appear as a danger-role notice below the control, so the field keeps its shape.
- **Gap:** numeric fields (`chunk duration`, `volume threshold`) are still browser default controls and are the one hole in this system.

## Do's and Don'ts

### Do:
- **Do** mix surfaces toward `--lift` (`{colors.lift}`), never toward literal `white`, so dark theme keeps a composed surface instead of an inverted one.
- **Do** put state in the left bar or the dot as well as the fill, and keep every state pair at 4.5:1 or better.
- **Do** give the action the deepest tone on the screen; the paper-white label on deep ochre is the one high-contrast moment.
- **Do** keep the display serif for headings and the sans for anything a user clicks or reads as data.

### Don't:
- **Don't** tint a surface ochre just for warmth; ochre means "the instrument is doing something".
- **Don't** invent a second red or a second green. `{colors.danger}` and `{colors.success}` are the only ones, in both themes.
- **Don't** add a second card shadow or a hover elevation to signal importance.
- **Don't** move toward the dark SaaS dashboard: near-black canvas, cyan or violet accents, glass panels, hero metrics.
- **Don't** put text on the mid-tone ochre (`{colors.ochre}`). It cannot clear AA with either light or dark text; that is why the action fill exists.
