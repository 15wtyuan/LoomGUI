# Preview simulation recipes

Consumer-layer (B) starting points. The behavior layer (component
expansion, control wiring, structural polyfill) is served by the preview
server itself from the running binary (`/yio-preview/lib/*`, auto-injected
boot entry — see SKILL.md). Everything below is what a workspace still
owns: demo data, fonts/theming, page navigation.

## pages/<page>.js — demo data

```js
// preview/pages/inventory.js — injected only into inventory.html.
import { ready } from '/yio-preview/lib/boot.js';
import { fillList, pageDir } from '/yio-preview/lib/fill.js';

const ICONS = ['item-potion', 'item-chest', 'item-gem'];
ready.then(() => {
  const dir = pageDir() + '../res/icons/';
  document.querySelectorAll('[role="list"][data-fill]').forEach((list) => {
    const count = parseInt(list.getAttribute('data-fill'), 10) || 8;
    fillList(list, count, (i, node) => {
      const img = node.querySelector('img');
      if (img) img.src = dir + ICONS[i % ICONS.length] + '.png';
    });
  });
});
```

`fillList(list, count, decorate)` clones the list's `<template>` until
`count` items exist (the template itself is item 0). Rect-diff tooling
removes these clones before measuring (core's static dump has no driver),
so fill freely — it never breaks the alignment gate.

Import both modules by absolute URL (`/yio-preview/lib/...`) — they are
version-matched to the running CLI, never copied into the workspace.

## main.js (optional) — fonts, theming, shared page glue

```js
// preview/main.js — injected into every page of the package (after boot).
const link = document.createElement('link');
link.rel = 'stylesheet';
link.href = 'preview/preview-theme.css';
document.head.insertBefore(link, document.head.firstChild);
```

`preview-theme.css` (name it anything) holds workspace-owned styling:

- Theme colors/backgrounds/decoration. Do **not** re-declare structural
  resets already owned by `/yio-preview/lib/base.css` (button reset,
  placeholder line) — same-name rules here would fight the
  framework copy as a second truth. (box-sizing needs no reset anywhere:
  the contract is CSS default content-box on both sides, #116.)
- Fonts need nothing here: the server auto-injects `@font-face` for every
  font in `yio.workspace.json` plus a default-family `body` rule
  (injected before your stylesheets, so your CSS wins). Hand-written
  `@font-face` is only for **overrides** — a different source file, or
  `font-display: swap` for a very large font (the injected rules use
  `block`). The family name must match the registered name exactly
  (unmatched families silently fall back to system fonts). `.ttc` files
  cannot be injected at all (browsers reject TrueType Collections) — the
  server skips them with a warning on stderr.

## Navigation & page-specific interaction (main.js)

```js
const NAV = { 'nav-settings': 'settings', 'nav-mail': 'mail' };
for (const [id, page] of Object.entries(NAV)) {
  const el = document.getElementById(id);
  el?.addEventListener('click', () => {
    location.href =
      location.href.substring(0, location.href.lastIndexOf('/') + 1) +
      page + '.html';
  });
}
```

Page-private interactions (battle replay, hotkey demos, readouts) follow
the same shape: plain DOM listeners on top of booted state. If a control
needs programmatic driving, import nothing extra — after `ready`, the
elements carry live attributes (`aria-valuenow`, `aria-expanded`,
`aria-checked`); mutate them and dispatch the matching Event the way the
A-layer wiring does.

## Adaptation check (full-bleed + notch) — run before handing off a page

The toolbar has a match-mode switcher and device presets with four-way
safe-area values; the dashed guides and the simulated `env()` values come
from the same table. For each page (new or restyled):

1. Mode `fit-width` (the common game base). Pick a notched preset
   (iPhone 14 / 16 PM) with safe-area guides on → the root background
   must reach the physical frame while interactive content sits inside
   the guide lines. Content outside the guides = missing root
   `env(safe-area-inset-*)` padding.
2. Switch to a different-ratio preset (4:3, 21:9, iPad mini) → content
   must reflow (scroll long pages, resize `vh`-sized stages), never clip
   or float; `vmin`-sized type scales with the frame.
3. Mode `letterbox` → the frame locks to the design resolution and black
   bars appear; env() values drop to 0 (the bars already yield). Use as
   the fixed-canvas control group.
4. Any fixed-px stage (model slots, effect slots) should be a `vh`
   height so it tracks the window; a px stage under fit modes is a bug
   the guides will not show — check the element list or resize.

## Trust list (what a preview can and cannot show)

- **Trustworthy**: flex layout, gap, px sizes, colors, gradient subset,
  `position:absolute`, `clip-path` shape masks
  (#52 — the browser renders circle()/polygon() natively with the same subset
  semantics: diagonal-normalized circle percentages, hit-testing through
  clipped-away areas, intersection with ancestor overflow clips, and clips
  rotating with the clipper's own transform),, `border-radius`, `@keyframes timing` (including inside
  component `<style>`; same-name collisions resolve page-wins, matching the
  packer's host priority), component expansion with scoped styles (server
  rewritten — root-class rules on the template root DO apply), control visual
  state, `cursor` (browser-native; mirrors the #93 runtime hand-intent
  default — in the Unity runtime the drawn hand depends on the host
  registering a cursor texture).
- **Approximate**: fonts (same files via @font-face, different rasterizer
  than the game), letterboxing (the shell scales per match_mode — check
  readability, not exact device pixels), page rules leaking into component
  subtrees (the browser has no style wall; the rewritten selectors carry a
  +0,1,0 specificity bump that keeps component rules winning equal-specificity
  contests — a higher-specificity page rule can still pierce).
- **Font loading note (#96)**: the server revalidates workspace static
  assets (`no-cache` + `304`), so after the first load fonts come from
  cache and refresh instantly. The FIRST load of a multi-MB font still
  transfers once: with `font-display: block` that window shows laid-out
  but invisible text (`!important` cannot help — it is not a cascade
  issue). Prefer `font-display: swap` for very large fonts, or keep
  `block` and accept a one-time flash on first visit.
- **Preview rejects like the build does (shown as CSS comments in the served
  sheet)**: non-`@keyframes` at-rules (`@media` …) and out-of-fence selectors
  inside component `<style>` — dropped, never silently applied.
- **Runtime-only (preview cannot show)**: NativeHost 3D projection,
  driver-driven list virtualization beyond demo fill, C# tween callbacks,
  focus/keyboard routing beyond the simulated bits.
- **Simulated end-to-end**: `env(safe-area-inset-*)` — the server rewrites
  it to CSS variables and the shell feeds device-preset values with the
  engine's formula (fit modes give real insets; letterbox gives 0 — its
  black bars already yield). Pick a notched preset (e.g. iPhone 14) to
  exercise avoidance; the dashed guides visualize the same numbers.
  Viewport units (`vw/vh/vmin/vmax`) resolve against the simulated root,
  so responsive font sizes and paddings flow when you switch device
  presets or match modes.

If the build emits a preview≠runtime warning (`FenceBorderWithoutStyle`,
`FenceBgImageWithoutSize`, non-transitionable `transition` properties,
`FenceDisplayInline`, dead sizing on inline text), the preview WILL lie
about exactly that property — fix the source, don't paper over it in a
preview script.
