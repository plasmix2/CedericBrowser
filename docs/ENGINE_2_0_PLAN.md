# Persona 2.0: in-house rendering engine plan

Status: **proposal, nothing implemented yet.** Written 2026-11-06. Numbers marked *(unverified)* come from
background knowledge or thin sources and must be measured before they drive a decision.

## 1. Goal and honest scope

Replace WebView2 (wry) with an in-process engine that uses much less memory per tab and starts faster.

A from-scratch engine that renders the modern web is a multi-year project, and the hardest parts are not
rendering but **JavaScript speed**, **web compatibility** (floats, tables, inline layout, ~hundreds of Web APIs)
and **security/site isolation**. So the plan assembles proven Rust components behind our own interface,
and lets measurements, not opinions, decide each layer. WebView2 stays as a fallback engine until the new
one is good enough, so users are never left with a browser that cannot open their sites.

Success is defined by numbers (section 6), not by "we wrote it ourselves".

**Priority order (set by the owner, 2026-11-06): speed first, and as useful as WebView2.** "Fastest everything"
and "lowest memory" pull in opposite directions (a JIT, GPU rasterization and caches all cost memory). Speed and
compatibility win ties; memory is minimized *within* that, mainly via process/tab discarding and shared caches.

## 2. Where we start (audit of this repo)

All engine coupling is in `src/main.rs` (`build_tab`, `Browser::*`), `src/devtools.rs` and `src/thumbs.rs`.
The surface is small: about 25 wry calls, 4 callbacks (navigation, title, page-load, new-window), an IPC
channel, `evaluate_script`, and the CDP loopback port. Engine-independent already: `toolbar.rs`, `gfx.rs`,
`popups.rs`, `pacer.rs`, `input.rs`, `suggest.rs`, `history.rs`, `crypto.rs`, `update.rs`, `favicons.rs`,
prefs/persona code, menus and tab-strip state.

The hard couplings:

| Coupling | Today | Needed from new engine |
|---|---|---|
| DevTools docking (`devtools.rs`) | CDP over loopback + Chromium inspector frontend | Own inspector, or a CDP server + bundled frontend |
| Thumbnails (`thumbs.rs`) | CDP `Page.captureScreenshot` | `snapshot()` straight from our own pixels (easier) |
| Injected JS (`SHORTCUT_SCRIPT`, `BRAND_SCRIPT`, ...) | Assumes Chromium web platform | Document-start scripts, `ipc.postMessage` shim; drop Speculation Rules / rely on native preconnect |
| Child-window layout, `present_with_damage` | Pages are child HWNDs over our softbuffer window | Disappears if the engine renders into our own surface |
| `BROWSER_ARGS` | Chromium flags | `EngineOptions` (autoplay, WebRTC leak policy, TLS min, QUIC) |
| PiP, fullscreen, permissions, profiles | wry/WebView2 features | Events and per-profile storage on the engine trait |
| `live_eval` test harness | CDP | `eval` on the trait |

## 3. Architecture

```
 Browser (main.rs, toolbar, popups ...)          unchanged UI
        |
   trait PageEngine / PageView                  Phase 0 seam
        |
   +----+-----------------------+
   | WebView2 backend (today)   |  Persona Engine backend (new)
   +----------------------------+
                                   net -> html5ever -> DOM -> Stylo (style)
                                   -> layout -> display list -> paint -> our surface
                                   JS engine + DOM/Web API bindings, event loop
```

Proposed trait (derived from actual usage; refine in Phase 0): `PageEngine::create_view(opts, event
callback, nav filter, permission callback)` and `clear_browsing_data(profile)`; `PageView` with
`load_url/reload/stop/go_back/go_forward/can_go_*`, `set_bounds/set_visible/set_memory_level/set_background`,
`focus/focus_parent`, `eval/eval_with_result`, `snapshot(width) -> Pixmap`, `devtools_frontend/open_devtools`.
Events: `Started, Finished, Title, NewWindow, Ipc, Pip, Fullscreen`. The security whitelist
(`Shortcut::parse`, `ContextRequest::parse`) stays host-side.

## 4. Component choices (candidates, to be decided by benchmarks)

| Layer | Primary candidate | Alternative | Notes |
|---|---|---|---|
| HTML parse | html5ever | - | Spec-complete, small |
| CSS cascade | Stylo | own cascade over cssparser | Stylo is heavy and MPL-2, but writing a cascade is a huge compat cost |
| Layout | Taffy (block/flex/grid) + own inline, float, table | Blitz layout | Blitz is pre-alpha, float support unconfirmed; Servo layout is not usable standalone |
| Text | cosmic-text (already used) | Parley | Avoid two shaping stacks |
| Paint | **GPU**: Vello or Skia (Graphite/Ganesh) on D3D12 via wgpu, compositor with GPU-resident tile layers | tiny-skia / vello_cpu as software fallback | Speed is the priority, so GPU is the default; GPU stack costs roughly 50-150 MB driver memory *(unverified)*, accepted. Bake-off picks Vello vs Skia |
| JavaScript | **V8 (JIT)** via the `v8` crate | QuickJS-ng only for tiny internal pages | Only option with JIT speed and full ES/Wasm support. Cost: ~20-30 MB binary, several MB per isolate *(unverified)*. Heavy `unsafe` stays in the renderer process |
| Network | hyper + rustls + h2, own cookie jar/cache | quinn+h3 later | HTTP/3 still unstable in the ecosystem |
| Whole engine fallback | Servo crate (0.x, monthly breaking releases, LTS branch) | WebView2 | Evaluate in Phase 1 as a bake-off |

**JavaScript is the make-or-break layer.** Given "fastest" and WebView2-level usefulness, the default is V8
(JIT, Wasm, full modern ES), with Phase 1 confirming its Windows build, isolate-per-site memory and embedding
cost. Interpreters (QuickJS-ng, Boa) are ruled out for general pages because they are too slow. Note that
V8 alone is not a web platform: DOM, fetch, workers, Web APIs, media, WebGL/WebGPU and WebRTC are all ours to build
or integrate, and are the real parity cost.

`unsafe_code = "forbid"` only covers our crate. JS engines, Stylo and graphics crates contain `unsafe`.
Plan: keep them in separate workspace crates / a renderer process, audit with `cargo-geiger` / `cargo-vet`,
and say so plainly in README (AGENTS.md rule 9).

## 5. Phases and agent workstreams

Each workstream is one branch, one owner, small PRs, per AGENTS.md. Claim it in INSTRUCTIONS.md first.

**Phase 0: seam (2-3 weeks, no behavior change).**
- WS-A *Trait extraction*: introduce `PageEngine`/`PageView`, move `build_tab` and the ~25 wry calls behind a
  WebView2 backend. Touches `main.rs` heavily: do this first, alone, to avoid conflicts.
- WS-B *CDP removal*: `snapshot()` and `eval` on the trait; make `thumbs.rs` and `live_eval` use them.
- WS-C *Benchmark harness*: reproducible memory (private bytes per tab, idle and loaded), startup, scroll
  frame time, Speedometer/JetStream-style JS scores, and a page corpus (top sites + simple documents).
  Baseline WebView2.

**Phase 1: de-risk (4-6 weeks, parallel, throwaway prototypes).**
- WS-D *V8 embedding spike*: `v8` crate on Windows, isolate/context cost, snapshotting for fast startup, a minimal DOM binding layer; compare to Boa/SpiderMonkey only as sanity checks.
- WS-E *Paint bake-off*: Vello vs Skia on D3D12 with a fixed display list; frame time, latency, GPU memory; tiny-skia as software fallback.
- WS-F *Servo bake-off*: embed the `servo` crate with software readback on our surface; measure memory/size/compat vs WebView2.
- WS-G *Compat gap study*: run html5ever+Stylo+Taffy prototype over the corpus; count what breaks (float, inline, tables).
- Gate: written go/no-go with numbers. If Servo or WebView2 meets the targets, we do not build layers ourselves.

**Phase 2: engine core (3-6 months).** Network + cache + cookies; DOM + event loop; style + layout (inline, float,
table); paint + scrolling + compositing into our surface; input/focus; JS bindings for DOM, fetch, timers,
storage, canvas. Renderer isolated in its own process with a sandbox.

**Phase 3: browser parity.** IPC shim and injected scripts, profiles/cookies, permissions, PiP/fullscreen,
media (audio/video codecs are a separate large item), native inspector replacing CDP docking, view-source.

**Phase 4: ship.** Opt-in flag (`PERSONA_ENGINE=native`), auto-fallback to WebView2 for sites that fail,
telemetry-free crash reports (existing `crash.log`), then default switch only when section 6 targets hold.

## 6. Targets (set after the Phase 0 baseline; proposed)

- Memory per background tab at least 50% below WebView2; idle browser well under the current total.
- Cold start and Ctrl+T faster than today's pre-loaded spare-tab hack (which can then be deleted).
- Scroll at display refresh on the corpus; JS within a stated factor of V8 on the chosen benchmark.
- Compatibility: pass rate on the corpus and a chosen WPT subset, with a published list of known failures.

## 7. Risks

1. **Web compatibility gap**: real sites break. Mitigation: corpus-driven development, WebView2 fallback.
2. **No JS engine is fast, small, Windows-friendly and safe at once**: decided by Phase 1 data.
3. **Servo / Stylo / Vello / Blitz are pre-1.0**: pin versions, budget for upgrades, track MPL-2 obligations.
4. **Security**: an in-process engine has no site isolation; one bug in a dependency's `unsafe` takes down the browser. Plan a renderer process and sandbox.
5. **Scope creep**: media codecs, WebRTC, WebGL, accessibility, IME, printing and PDF are each large; cut explicitly in a scope list.
6. **Merge collisions** with the other agent in `main.rs`/`toolbar.rs`: do WS-A first and alone.

## 8. Decisions

Decided (owner, 2026-11-06): speed is the top priority and the engine must be as useful as WebView2, so the JS engine is V8 (JIT)
and painting is GPU-first.

Still open:
1. Is a WebView2 fallback acceptable during and after the transition? (Plan assumes yes; without it, "as useful as WebView2" cannot be guaranteed at launch.)
2. Is Servo acceptable as the 2.0 engine if the bake-off shows it beats a custom stack on speed and compat?
3. Target platforms beyond Windows?
