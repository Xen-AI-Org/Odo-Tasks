# Design — Odo

A locked design system for the Odo desktop app. The selected reference is the utilitarian cream workspace generated on July 18, 2026. Every product surface uses this system.

## Genre

Modern-minimal with a calm utilitarian tone.

## Macrostructure family

- App pages: Workbench — compact application sidebar, spanning command strip, focused content surface, contextual inspector where useful.
- Content pages: Long Document — generous reading width, quiet rules, serif titles, sans-serif body controls.
- Marketing pages: not in scope.

## Theme

All colors are official Tailwind CSS palette values.

- Paper: `stone-50`, `stone-100`, and white.
- Ink: `stone-950`, `stone-700`, and `stone-500`.
- Rules: `stone-200` and `stone-300`.
- Brand/action: `amber-500`, with `amber-600` for pressed emphasis and `amber-50` for selection.
- Completion: `emerald-600` and `emerald-700`.
- Destructive: `red-600` only.

## Typography

- Display: Newsreader Variable, weight 400–550, roman only.
- Body: Geist Variable, weight 400–650.
- Mono: Geist Mono or system monospace for shortcuts and timestamps.
- App controls stay at 13–15px; editor body stays at 15–16px.

## Spacing

4-point named scale in `tokens.css`. App density is compact-comfortable: 8px row gaps, 12–16px control padding, 20–24px section padding.

## Motion

- State changes use opacity and transform only.
- `--ease-out` is the default easing.
- Reduced motion collapses to opacity-only transitions of 150ms or less.

## Microinteractions stance

- Silent save and success states.
- Instant keyboard focus rings using `amber-500`.
- Hover reveals secondary row actions; primary actions remain visible.
- Destructive actions require the existing Odo confirmation dialog.

## Component voice

- shadcn/ui `new-york` components with Radix primitives.
- Low 8px radius, crisp `stone-200` borders, minimal shadow.
- Lists are grouped surfaces with separators, not collections of floating cards.
- Command search, Tabs, Checkbox, DropdownMenu, Tooltip, Button, Input, ScrollArea, and Separator establish the shell.

## Per-page allowances

- App pages must not use illustration or decorative imagery; function carries the page.
- Notes may use a serif title, but navigation and body controls remain Geist.
- Project, task, journal, and settings views share the same sidebar, command bar, tokens, and component density.

## What pages MUST share

- Odo wordmark.
- `stone` paper system and ≤5% amber per viewport.
- Geist UI typography and Newsreader title typography.
- 8px component radius, thin rules, and compact toolbar rhythm.
- Visible focus states and reduced-motion support.

## Exports

### tokens.css

The canonical runtime export is [`tokens.css`](./tokens.css).

### Tailwind v4

`tokens.css` exposes the semantic `@theme inline` mapping used by Tailwind utilities and shadcn/ui components.

### DTCG

The portable token file is [`tokens.json`](./tokens.json).

### shadcn/ui CSS variables

- `background` → `stone-50`
- `foreground` → `stone-950`
- `primary` → `amber-500`
- `primary-foreground` → `stone-950`
- `secondary` and `muted` → `stone-100`
- `border` and `input` → `stone-200`
- `ring` → `amber-500`
- `destructive` → `red-600`
- `radius` → `0.5rem`

## Final quality gate

Slop test: passed. The selected utilitarian workbench keeps a single Phosphor icon language, uses no gradients or decorative filler in the redesign layer, clips page-edge overflow, and was visually checked at 320px, 375px, 414px, 768px, and 1440px.
