# Persona 2.0: in-house rendering engine plan

Status: **proposal, nothing implemented yet.** Written 2026-11-06. Numbers marked *(unverified)* come from
background knowledge or thin sources and must be measured before they drive a decision.

## 1. Goal and honest scope

Replace WebView2 (wry) with an in-process engine that uses much less memory per tab and starts faster.

A from-scratch engine that renders the modern web is a multi-year project, and the hardest parts are
**JavaScript speed**, **web compatibility** and **security/site isolation**. **Decision (owner, 2026-11-06): use
[Servo](https://servo.org) as the base and add on to it where needed.** Servo already provides HTML parsing
(html5ever), styling (Stylo), layout, WebRender, a SpiderMonkey JS engine, networking, DOM and Web APIs, and
ships as an embeddable crate. We write the browser around it, upstream fixes where we can, and keep patches to a
minimum. Servo is pre-1.0 and less compatible than Chromium, so WebView2 stays as a fallback engine until
measurements say otherwise.

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
 Browser (main.rs, toolbar, popups ...)          unchanged native UI
        |
   trait PageEngine / PageView                  Phase 0 seam
        |
   +----+--------------------------+
   | WebView2 backend (today,      |  Servo backend (new, default when it passes the gates)
   | kept as fallback)             |  servo crate (LTS branch) + Persona patches/add-ons
   +-------------------------------+
```

Proposed trait (derived from actual usage; refine in Phase 0): `PageEngine::create_view(opts, event
callback, nav filter, permission callback)` and `clear_browsing_data(profile)`; `PageView` with
`load_url/reload/stop/go_back/go_forward/can_go_*`, `set_bounds/set_visible/set_memory_level/set_background`,
`focus/focus_parent`, `eval/eval_with_result`, `snapshot(width) -> Pixmap`, `devtools_frontend/open_devtools`.
Events: `Started, Finished, Title, NewWindow, Ipc, Pip, Fullscreen`. The security whitelist
(`Shortcut::parse`, `ContextRequest::parse`) stays host-side.

## 4. What Servo gives us and what we add

| Area | From Servo | Persona add-on / risk |
|---|---|---|
| Parse, style, layout, paint | html5ever, Stylo, Servo layout, WebRender | Compatibility gaps vs Chromium; report/fix upstream first, patch only if blocked |
| JavaScript | SpiderMonkey (mozjs), with JIT | Not V8: speed and Web API behavior differ from WebView2. Windows build of mozjs is painful (MozillaBuild); use prebuilt/cached artifacts in CI |
| Rendering into our window | Servo rendering contexts (software readback or GPU/window) | Must integrate with our softbuffer UI and remove the child-HWND/`present_with_damage` workaround; spike in Phase 1 |
| Networking | Servo net stack | Need cookies per persona profile, cache, TLS/QUIC policy, HTTPS upgrade; `input::is_allowed_navigation` stays as a navigation filter |
| Events/IPC | Embedder delegate callbacks | Re-create the `window.ipc.postMessage` shim and keep the host-side whitelist; document-start script injection |
| Profiles, permissions, clear-data | Partial | Persona profiles, permission callback, clear-site-data: likely add-ons |
| DevTools | Servo devtools server (Firefox remote protocol, not CDP) | Our docked pane, thumbnails and `live_eval` can't use CDP: thumbnails become `snapshot()`, inspector needs a Firefox-protocol client/frontend or a native one |
| PiP, fullscreen, media | Partial, codecs depend on build | Verify video/audio codecs, PiP and background playback; may need add-ons |
| Chromium-only features in our scripts | none | `speculationrules`, Chromium PiP API: drop or reimplement host-side |

Servo is MPL-2: modified Servo files must stay MPL-2 and available. `unsafe_code = "forbid"` only covers our
own crate; Servo, SpiderMonkey and Stylo contain `unsafe`. Say so plainly in README (AGENTS.md rule 9) and, if
Servo's multi-process/sandbox mode is usable on Windows, enable it for site isolation.

Use Servo's **LTS branch** for shipping and pin exact versions; the monthly releases may break the embedding API.

## 5. Phases and agent workstreams

Each workstream is one branch, one owner, small PRs, per AGENTS.md. Claim it in INSTRUCTIONS.md first.

**Phase 0: seam (2-3 weeks, no behavior change).**
- WS-A *Trait extraction*: introduce `PageEngine`/`PageView`, move `build_tab` and the ~25 wry calls behind a
  WebView2 backend. Touches `main.rs` heavily: do this first, alone, to avoid conflicts.
- WS-B *CDP removal*: `snapshot()` and `eval` on the trait; make `thumbs.rs` and `live_eval` use them.
- WS-C *Benchmark harness*: private bytes per tab (idle/loaded), startup, scroll frame time, JS benchmarks, a page
  corpus (top sites + simple documents). Baseline WebView2.

**Phase 1: Servo feasibility (4-6 weeks, parallel spikes).**
- WS-D *Embed Servo on Windows*: build the `servo` crate (LTS) in CI, create a view with software or GPU readback into our surface, load pages, input and focus.
- WS-E *Compat and speed run*: run the corpus and a WPT subset on Servo vs WebView2; list failures by cause (layout, JS, Web API, media).
- WS-F *Integration gaps*: IPC shim, document-start scripts, navigation filter, profiles/cookies, permissions, devtools options.
- Gate: written go/no-go with numbers. If Servo cannot meet the targets, we reconsider before investing further.

**Phase 2: Servo backend.** Implement `PageEngine` on Servo; tab lifecycle (spare tab, suspend/discard), history, favicons, title, new-window, context-menu, view-source.

**Phase 3: parity add-ons.** Close the gaps from WS-E/F: profiles, permissions, PiP/fullscreen, media, inspector/thumbnails without CDP, installer size and update flow. Upstream patches first.

**Phase 4: ship.** Opt-in flag (`PERSONA_ENGINE=servo`), automatic fallback to WebView2 for sites that fail, existing `crash.log`, then switch the default only when section 6 holds.

## 6. Targets (set after the Phase 0 baseline; proposed)

- Memory per background tab at least 50% below WebView2; idle browser well under the current total.
- Cold start and Ctrl+T faster than today's pre-loaded spare-tab hack (which can then be deleted).
- Scroll at display refresh on the corpus; JS within a stated factor of V8 on the chosen benchmark.
- Compatibility: pass rate on the corpus and a chosen WPT subset, with a published list of known failures.

## 7. Risks

1. **Web compatibility gap**: Servo trails Chromium; real sites will break. Mitigation: corpus-driven testing, WebView2 fallback, upstream contributions.
2. **SpiderMonkey, not V8**: JS speed and behavior may fall short of WebView2; measured in WS-E. Swapping the JS engine inside Servo is not realistic.
3. **Servo is pre-1.0**: monthly breaking embedding API; use LTS, pin versions, budget for upgrades and security patches.
4. **Memory and binary size**: Servo is much bigger than our current ~4 MB exe *(unverified)*; speed is the priority but we measure and report it.
5. **Security**: unless Servo's sandbox/multi-process works on Windows, there is no site isolation. Verify in Phase 1.
6. **Windows build and CI time**: mozjs and Stylo are slow to build; cache artifacts.
7. **Scope creep** and **merge collisions** in `main.rs`/`toolbar.rs`: do WS-A first and alone.

## 8. Decisions

Decided (owner, 2026-11-06): speed is the top priority, the engine must be as useful as WebView2, and **Servo is the base**
with add-ons where needed (this replaces the earlier V8/GPU-stack proposal).

Still open:
1. Is a WebView2 fallback acceptable during and after the transition? (Plan assumes yes.)
2. Target platforms beyond Windows?
