# CSS reference

Supported properties (complete whitelist; everything else is a build
error):

<!-- fence-sync:css-supported-begin -->
- `width` / `height` / `min-width` / `min-height` / `max-width` / `max-height` — px, %, auto, viewport units (`vw` / `vh` / `vmin` / `vmax` — resolve against the stage canvas, not the parent; the reflow language for resolution adaptation)
- `display` — block / flex / none / inline (grid rejected)
- `flex-direction` / `flex-wrap` / `flex-grow` / `flex-shrink` / `flex-basis`
- `gap` / `row-gap` / `column-gap` — px or viewport units (1-2 values)
- `justify-content` / `align-items` / `align-content` / `align-self`
- `order` / `aspect-ratio` — integer
- `z-index` — integer, no `auto`; declared only on positioned elements or flex items (build error elsewhere — see the z-index bullet under Positioning)
- `position` — absolute / relative / static (initial value `static`); with `top` / `right` / `bottom` / `left` (px / % / auto / viewport units / `env()`; `%` resolves against the containing block — browser semantics)
- **Containing block of an `absolute` element = nearest ancestor with `position: relative` or `absolute`** (browser semantics); if none, the viewport. Known limits: an `absolute` element with all four insets `auto` keeps its direct-parent static position (browser hypothetical-box semantics not implemented); overflow clipping still follows DOM ancestors.
- `padding-top` / `padding-right` / `padding-bottom` / `padding-left` — px or viewport units
- `margin-top` / `margin-right` / `margin-bottom` / `margin-left`
- `border-color` / `border-style` / `border-radius` (px / % / viewport units) / `border-image-slice`
- `background-color` / `background-image` / `background-size` / `background-repeat` / `background-clip` / `-webkit-background-clip`
- `opacity` / `box-shadow` / `pointer-events` / `transform` / `transform-origin` / `filter`
- `cursor` — `auto` / `default` / `none` / `pointer` (not inherited). `auto` = UA default:
  hovering pressable controls (button / tab / toggle / radio / slider / dropdown /
  option, and `<a>` links) reports pointer-hand *intent* at runtime; everything else
  stays the system arrow. Hovering a control's text/inline children counts as hovering
  the control (browser-consistent), and disabled controls never get the hand.
  Explicit declarations always win over the UA default — use
  `pointer` to mark clickable non-controls (map nodes etc.), `default` to force an
  arrow on a control, `none` for element-level software-cursor hiding. Browser
  preview renders `cursor` natively, so there is no preview gap. In the Unity
  runtime the hand is drawn only if the game registers a cursor texture
  (`YioStageDriver.SetCursorTexture`); unregistered, hover falls back to the
  system arrow.
- `color` / `font-size` (px or viewport units — responsive sizes like `font-size: 2vmin`;
  `%` / `em` / `rem` rejected) / `font-family` / `font-weight`
- `text-align` / `line-height` / `letter-spacing` (px or viewport units) / `white-space` / `text-shadow`
- `white-space` — full set: `normal` / `nowrap` / `pre` / `pre-wrap` / `pre-line` (space collapsing × auto-wrap × source-newline preservation); CJK line breaking avoids line-start punctuation / line-end opening brackets (kinsoku)
- `overflow-wrap` — `normal` (overlong word overflows, browser-consistent) / `break-word` (split only when the word alone exceeds the line)
- `word-break` — `normal` / `break-all` (break between any characters) / `keep-all` (no breaks inside CJK words)
- `text-wrap` — `normal` / `nowrap` (disables soft wrap; `balance` / `stable` / `pretty` are rejected — use `text-align` for centered headings)
- `text-decoration` — `none` / `underline` only (not inherited; `<a>` UA default is `underline`, author declarations override)
- `-webkit-text-stroke` / `font-effect` — Yio text extensions
- `caret-color` / `selection-background` / `selection-color` / `placeholder-color` / `-webkit-text-security` — text-control theming
- `animation` and longhands: `animation-name` / `animation-duration` / `animation-timing-function` / `animation-delay` / `animation-iteration-count` / `animation-direction` / `animation-fill-mode` / `animation-play-state`
- `transition`
- `overflow-x` / `overflow-y` — visible / hidden / scroll / auto
- `clip-path` — geometric shape mask (fence subset): `none` |
  `circle(<length|%> [at <length|%> <length|%>])` |
  `polygon(<x> <y>, ...)` with 3..=16 points. Circle `%` radius resolves
  against `sqrt(w^2+h^2)/sqrt(2)` (exact CSS semantics — `circle(50%)` inscribes
  a square box); polygon `%` resolves against width/height per axis. Declaring
  it makes the element a clipper for its own paint AND its subtree (web
  semantics), independent of `overflow`; hit testing respects the shape
  (clipped-away areas click through). Hard-rejected combos:
  `overflow:scroll/auto` + `clip-path`, clip chains deeper than 4 nested
  clippers. `ellipse()` / `inset()` / `closest-side` / `farthest-side` /
  `fill-rule` prefixes / geometry-box keywords are outside the fence.
- `resize` — accepted as a no-op (never consumed)

Shorthands (expand to the properties above):

- `padding` — four-side box
- `margin` — four-side box
- `inset` — four-side box (top / right / bottom / left)
- `overflow` — sets both axes
- `border` — color-led border shorthand
- `border-width` — four-side box
- `border-top` — single side
- `border-right` — single side
- `border-bottom` — single side
- `border-left` — single side
- `background` — color, image, size, repeat
- `flex` — grow, shrink, basis
<!-- fence-sync:css-supported-end -->

Length tokens (wherever a length is accepted): besides px / % / viewport units,
`env(safe-area-inset-top)` / `env(safe-area-inset-right)` /
`env(safe-area-inset-bottom)` / `env(safe-area-inset-left)` resolve to the
depth the UI canvas reaches into the unsafe screen region (design px). Under
full-bleed adaptation (fit-width / fit-height) the canvas covers the physical
screen, so these give the notch / home-indicator margins — combine with
`padding` or `inset` to keep content clear. Under letterbox the canvas lives
entirely inside the safe area, so all four are `0` (the black bars already
yield; no double avoidance). Other `env()` names are build errors. Browser
preview rewrites these to CSS variables fed from the selected device preset,
so preview matches the runtime.

## Value domains

- `background-image`: `none`, `url(...)`, `linear-gradient` /
  `radial-gradient` (up to 8 stops, hex / `rgb()` / `rgba()` colors).
  `conic-gradient` and `repeating-*` variants do not exist.
- `background-size`: `cover` / `contain` / `100%` / `stretch`.
- `filter`: grayscale / brightness / contrast / saturate / hue-rotate /
  invert / sepia.
- `transform`: translate / rotate / scale. `transform-origin` sets the
  pivot (two `<length|%>` values or position keywords
  `left/center/right/top/bottom`; default `center`) — rotate around a
  non-center point directly, no need to offset-position the element at
  an arc midpoint.
- `transition`: animates exactly these channels (layout channels
  interpolate same-unit explicit endpoints; `box-shadow` interpolates
  per layer):
  <!-- fence-sync:transition-channels-begin -->
  - `background-color`
  - `color`
  - `opacity`
  - `transform` — decomposed translate-scale-rotate interpolation
  - `width`
  - `height`
  - `flex-grow`
  - `box-shadow`
  <!-- fence-sync:transition-channels-end -->
  Everything else (`margin`, `filter`, ...) changes instantly; the build
  warns per property.
- `line-height`: unitless multiplier (`1.5`), `px`, or `normal` (`em` and
  `%` are build errors). A bare unitless number is a **multiplier**, not
  pixels — `line-height: 26` at `font-size: 16` gives a 416px line; write
  `26px` for pixel line height.
- `white-space` (whitespace folding pitfall): under `normal` / `nowrap`
  / `pre-line`, runs of spaces and **source-code newlines** in static
  text collapse to a single space (browser semantics) — a multi-line
  HTML text block renders as one flowed paragraph, not line breaks.
  To preserve formatting use `pre-wrap` (keep spaces + newlines, wrap)
  or `pre-line` (collapse spaces, keep newlines); `pre` also disables
  wrapping. CJK text line-breaks with kinsoku (no line-start
  punctuation / line-end opening brackets) automatically.
- `position`: `absolute` / `relative` / `static` — `fixed` and `sticky` are build
  errors.
- `z-index`: integer only, no `auto`. **Declaration site is checked (build
  error)**: z-index is only valid on a positioned element
  (`position:relative/absolute`) or on a flex item (child of a
  `display:flex` container) — anywhere else browsers ignore the declaration
  while the runtime honors it, so the fence rejects it. Pair every z-index
  with `position` (or a flex parent).
- **Paint-order lint (warning)**: when static and positioned (or z-declaring)
  siblings share a parent and the static side declares no z-index, `yio
  check` warns — positioned elements always paint above static content
  regardless of DOM order, so an undeclared order only works by luck. To
  silence it when overlap is intended, declare the intent explicitly:
  `position:relative; z-index:0` on the static element (same visual).

## Selectors

A selector is a chain of compounds joined by combinators: whitespace
(descendant — matches at any ancestor depth) or `>` (child — matches the
direct parent only). Each compound is `tag? (.class | #id | [attr] | :pseudo)*`:

- Pseudo-classes that work: `:hover`, `:active`, `:focus`, `:disabled`,
  `:checked`, `:nth-child(An+B | odd | even | N)`. They gate on live
  interaction state and re-evaluate every frame — `:hover` driven
  styling needs no runtime class toggling.
- `::part(name)` — the only pseudo-element in the fence: a page rule
  pierces the component content wall and targets nodes inside the
  component that carry `part="name"`. In `prefix::part(name)`, the
  compound prefix (class/tag/pseudo) matches the component **host**
  (hosts live in page scope and can carry class/id); the part matches
  nodes in that host's expanded subtree. It must end the last compound
  (`X::part(a) Y` is a build error), does not recurse into nested
  components (their internals belong to the nested host), and
  specificity follows the web (part name as attribute + pseudo-element:
  `.card::part(title)` = (0,2,1)). Also usable in runtime
  `StyleSheet.Add` (runtime rules already pierce literally; `::part`
  there is just a matching arm). The preview server rewrites `::part(n)`
  to the flat `[part="n"]` descendant equivalent so browser preview
  shows the styling too. **Authoring rule:** keep overridable properties
  (color and friends) off inline `style` on part-target nodes — inline
  declarations beat every class rule including `::part` (the page can
  never win); put the component's default visuals in `<style>` classes
  and reserve inline for structural layout.
- Build errors (the diagnostic names the offending construct):
  combinators `+` / `~` (`>` child is supported; adjacent and general
  siblings are not), the universal selector
  `*`, unknown pseudo-classes (`:not()`, `:nth-of-type`, ...), and
  pseudo-elements other than `::part` (`::before`, `::after`, ...).
- Attribute selectors: `[attr]` and `[attr="value"]` only; higher
  operators (`^=`, `~=`, `$=`, `*=`, `|=`) are build errors.
- Do not use `:nth-child` on virtualized lists (`role=list` bound to
  data): parked slots count as children and skew the index — use
  `[data-index="N"]` instead.

<!-- fence-sync:css-not-supported-begin -->
Properties that do NOT exist in the fence (using any of these is a
`FenceUnknownCssProp` build error):

- `box-sizing` — there is no border-box switch; padding adds to the set width/height
  (width:420px with padding:22px renders 464px wide). Full-bleed page roots:
  keep `width:100vw; height:100vh` with **zero padding** and inset content via
  the sections inside — a padded root overflows the canvas exactly like it
  would in a browser.
- `font-style` — no italic via CSS (and no `em` / `i` tags either)
- `text-transform`
- `user-select` — use `pointer-events` for interaction gating
- `vertical-align`
- `float`
- `background-position`
- `object-fit`
- `text-overflow`
- `list-style`
<!-- fence-sync:css-not-supported-end -->

## Custom properties and `var()`

Declare theme tokens as CSS custom properties and consume them with
`var()` in any property's value:

```css
.theme { --accent: #2a5a75; --line-strong: var(--accent); }
.card  { color: var(--accent); border-color: var(--line-strong, #888); }
```

- **Sources (web semantics, three)**: `<style>` rule declarations,
  inline `style="--accent: #f00"`, and the runtime C# API
  `Style.SetVar` (highest priority; `RemoveVar` falls back to the CSS
  value). The HTML attribute `--*` is passthrough data only (matchable
  by `[attr]` selectors) — it is NOT a var() source.
- `var(--x, fallback)` fallback is supported and may nest `var()`.
  Values containing `var()` skip literal validation at build time (the
  final value is only known at runtime); malformed shapes (unbalanced
  parens, non-`--` names) are build errors.
- Custom properties inherit down the tree and cross component
  boundaries; resolution happens at the declaring element (a `--a:
  var(--b)` resolves `--b` in the declaring element's environment).
- **Cycles and missing targets**: a reference cycle (`--a ↔ --b`) or a
  broken chain makes that property invalid — declarations using it are
  skipped (fallback still applies) with a runtime warn-once log, never
  an exception. The build warns for cycles statically visible within
  one `<style>` block or one `style` attribute
  (`FenceCustomPropCycle`). A missing target without fallback is legal
  (runtime `SetVar` may inject it later).
- Runtime injection (`UIContext.StyleSheet.Add`) accepts the same
  selector+declaration subset INCLUDING `--*` / `var()`; at-rules are
  rejected (parse failure throws `UIStyleException` with line/column).

## Animations

Define with `@keyframes <name> { from {...} to {...} 50% {...} }` inside
`<style>`, apply via the `animation` shorthand (`<name> <duration>
[easing] [count|infinite] [fill-mode] [direction] [delay]`, e.g.
`animation: fadeIn .4s .05s both`). Identical duplicate keyframes across
component instances merge silently; same name with different content
warns (host wins) — prefer defining shared animations page-level or in a
shared external CSS and referencing the name from components.

## Browser-difference traps (preview honesty)

The build flags every one of these with a warning; with zero warnings the
browser preview is honest.

- `background-image` without `background-size`: browsers show the
  original size, Yio stretches to fill.
- `border-width` without `border-style`: browsers draw nothing, Yio
  draws the border.
- Adjacent margins never collapse (browsers collapse them vertically);
  prefer `gap` for spacing.
- No inline flow outside flex containers and rich-text blocks.
