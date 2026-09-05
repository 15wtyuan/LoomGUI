---
name: yio-preview
description: |
  Write browser preview simulation scripts for Yio pages and run the
  human preview loop. Use when a Yio page is finished and a human needs
  to see it (`yio preview` workbench), when `yio check` reports
  PreviewDataFillWithoutSim, or when a preview script (preview/main.js,
  preview/pages/<page>.js) needs writing or fixing. The server ships the
  behavior layer (component expansion, control semantics) itself; your
  scripts are workspace-owned consumer-layer code only.
---

# Yio Preview Simulation

Pages render statically in the browser; runtime behavior does not exist
there. The preview server closes that gap in **two layers** (#92):

- **A layer — shipped by the framework.** For every HTML page the server
  auto-injects `/yio-preview/lib/boot.js`, which performs component
  expansion (custom elements, slot projection, host-state mirroring) and
  injects each component's scoped stylesheet — a server-rewritten copy
  (`/yio-preview/comp-style/<name>.css`, single truth in Rust) that matches
  core's style-wall semantics, including root-class rules on the template
  root. It also wires control semantics (slider/combobox/switch/spinbutton/
  tabs/trees/dialogs/progressbar/textbox) and injects the structural base polyfill
  (button reset, placeholder line; no box-sizing reset — the contract is CSS
  default content-box on both sides, #116). Its content is embedded
  in the running `yio` binary — always version-matched to the CLI. You
  never copy or reimplement any of this; a workspace copy would rot into a
  second source of truth (this happened in dogfooding; it is now a design
  error, not a recipe).
- **B layer — yours.** Demo data, page navigation, page-specific
  interactions, theming. These live in `preview/main.js`
  (per-package, optional) and `preview/pages/<page>.js` (per-page); the
  server injects them after boot when the files exist. Fonts are NOT
  your layer: the server auto-injects `@font-face` from the workspace
  font registry (see below), so preview and Unity runtime resolve the
  same families from the same files.

Rendering parity between browser and runtime is a framework guarantee;
your scripts restore **consumer behavior**, never re-layout.

## The mechanism (what the server does for you)

`yio preview <workspace>` (long-running; prints one JSON with `url`) serves
a workbench: package/page tree on the left, device-frame preview on the
right. Scaling follows the match mode (switchable in the toolbar,
persisted): `letterbox` locks the iframe to the design resolution (no
reflow, black bars); `fit-width` / `fit-height` size the iframe to the
runtime root (the free axis reflows with the device frame — vw/vh
denominators follow, exactly like the engine). Per page it injects at
most three ES modules, in this order:

```
/yio-preview/lib/boot.js        ← ALWAYS (framework behavior layer)
<package-dir>/preview/main.js    ← if present (shared consumer sim)
<package-dir>/preview/pages/<page>.js  ← if present (demo data)
```

Wait for A layer before touching filled DOM:

```js
import { ready } from '/yio-preview/lib/boot.js';
ready.then(() => { /* fill lists, drive page state */ });
```

HTML sources stay clean (zero `<script>` references); the `preview/`
directory never enters the build. Modules are deferred — the DOM is parsed
when they run. Server restarts reuse a stable port, so an open tab
survives (human just refreshes).

## Font injection (automatic — #104)

Every served page also gets a `<style id="yio-preview-fonts">` right
after `<head>`, generated from the workspace font registry
(`yio.workspace.json` → `fonts`):

- one `@font-face` per registered font (`src: url(/ws/<file>)` — works
  from any page depth);
- a `body { font-family: '<default-family>' }` rule mirroring the
  runtime's "no font-family declared → default font" semantics;
- injected **before** the page's own `<style>`/`<link>`, so workspace
  CSS always overrides the default-family rule.

Limits the server warns about on stderr (and skips the entry): `.ttc`
(browsers cannot load TrueType Collections) and registry files missing
on disk. If a page falls back to system fonts, check the server output
first.

## When you MUST write a script

- Page has `data-fill` lists (runtime-populated ListView) → the human
  preview shows empty lists otherwise. `yio check` warns
  `PreviewDataFillWithoutSim` when the per-page script is missing — that
  warning is your cue.
- Page is driven by game code (readouts, dynamic content) → a per-page
  script simulating that data makes the preview honest.
- Page has custom navigation/hotkeys/scene-specific behavior → wire it in
  `main.js`.

A workspace with none of the above needs **no scripts at all** — component
pages and control pages are already alive through the A layer.

## Workflow (the human preview gate)

1. Write/fix the page HTML/CSS until `yio check` exits 0.
2. Write the simulation scripts (recipes in `references/recipes.md`).
3. Start or reuse the server: `yio preview` from the session root — if
   one is already running it prints the same URL (`reused: true`); find it
   in `.yio/preview.json` if stdout was swallowed by backgrounding.
4. Give the human the URL. They refresh (F5) after each of your edits —
   the server reads sources live, no restart needed.
5. Iterate on feedback; only after the human approves the preview does the
   page proceed to `yio build` / runtime wiring. Human preview is the
   gate before handing off to Unity.
6. Stop the server with `yio preview --stop` when the session is done
   (it also self-exits after 4h idle).

## Boundaries

- Never reference preview scripts from page HTML (the fence owns the
  document; injection is the server's job).
- Never reimplement what the A layer owns (expansion, controls) — you
  cannot out-source-of-truth the framework; import `ready` from the boot
  module instead.
- Never simulate what the framework guarantees (flex layout, px math,
  gradients, keyframes timing) — scripts restore behavior, not pixels.
- Custom resolutions / safe-area guides are preview-shell UI (localStorage
  prefs); they never touch the workspace.
- `env(safe-area-inset-*)` is rewritten server-side to
  `var(--yio-safe-*, 0px)`; the shell feeds the vars from the selected
  device preset using the same formula as the engine (fit = real insets
  against the physical frame, letterbox = 0). Safe-area guides and env
  values share one preset table.
- Trust tiers for what a preview can and cannot show:
  `references/recipes.md` §Trust list.

## References

| File | Contents |
|---|---|
| `references/recipes.md` | copy-paste recipes: demo data fill, fonts/theming, navigation, trust list |
