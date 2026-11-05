# Rules for AI agents

This file is for any AI coding agent working in this repository (Claude, Gemini, Codex, Copilot, Cursor…).
**Read it fully before changing anything. These rules are mandatory, not suggestions.**

More than one person, and more than one agent, works on Cederic. In the past an agent rewrote whole
files from an old copy and silently deleted another agent's work. Everything below exists to stop that
from happening again.

**Multi-agent coordination:** Before starting any task and before finishing/merging, you **must** also read
and update [INSTRUCTIONS.md](INSTRUCTIONS.md). That file is the live communication board and automatic
sync/merge protocol shared between the two agents working on this repo.

## Hard rules

1. **Never work on `main` directly.** Create a branch (`git switch -c <you>/<short-topic>`), commit there,
   and open a pull request. A human merges it.
2. **Never force-push, rebase shared branches, or rewrite history.** No `git push --force`,
   `git reset --hard` on pushed commits, or `git commit --amend` on pushed commits.
3. **Start from the latest code.** Run `git fetch` and `git status` first, and branch from the current
   `origin/main`. If `git status` shows changes you did not make, **stop and ask the human**. Another
   agent or person may be mid-task in the same folder.
4. **Never rewrite a whole file from memory or from an old copy.** Read the file as it is now, then make
   targeted edits. Replacing a whole file is only acceptable for a file you created in this same task.
5. **Do not delete or "simplify away" features you don't understand.** Everything in
   [What must keep working](#what-must-keep-working) is intentional. If you think something should go,
   ask the human first.
6. **Keep changes small and on-topic.** One feature or fix per branch. Don't reformat or reorganise
   unrelated code.
7. **Before every commit:** `cargo build --release` with no errors and no warnings, then `cargo test`
   with everything passing. Say in the PR what you could not test.
8. **Do not commit** `target/`, secrets, tokens, personal data, or the user's profile folder
   (`%LOCALAPPDATA%\Cederic`).
9. **Be honest in the UI and in docs.** Don't add badges, panels or README claims about security or privacy
   features unless the code actually implements them and you checked that it works.

## Project overview

Cederic is a Windows browser written in Rust. The window, the browser UI and all browser logic are
Rust (`unsafe` is forbidden in `Cargo.toml`); only web pages are rendered by the system's WebView2
engine, through [wry](https://github.com/tauri-apps/wry) and [tao](https://github.com/tauri-apps/tao).
The whole browser UI (tab strip, toolbar, menus, suggestions, hover cards) is **not** HTML: it is drawn
by our own code with
[tiny-skia](https://github.com/linebender/tiny-skia) and [cosmic-text](https://github.com/pop-os/cosmic-text)
and shown with [softbuffer](https://github.com/rust-windowing/softbuffer).
(This is moving to the ANVL SDK; see [Native UI: moving to the ANVL SDK](#native-ui-moving-to-the-anvl-sdk).)

```
cargo run --release                      # opens a new tab
cargo run --release -- example.com       # opens a URL or search
cargo test                               # unit tests
```

Note that building replaces `target/release/Persona.exe`, so any running Persona must be closed first.

| File | What it does |
|---|---|
| `src/main.rs` | Window, tabs, DevTools pane, event loop; decides what menus/suggestions/cards show |
| `src/toolbar.rs` | Native tab strip, window controls, buttons and address bar: layout, input, animation, drawing |
| `src/gfx.rs` | Drawing helpers for the native UI: shapes, vector icons, text, theme colors |
| `src/popups.rs` | Native popup windows: right-click menu, suggestion dropdown, tab hover card |
| `src/pacer.rs` | Frame timing for animations (see below) |
| `src/favicons.rs` | Fetches and decodes tab favicons (PNG/ICO) for the native tab strip |
| `src/input.rs` | Address-bar input → URL or search; HTTPS upgrades; search-query extraction |
| `src/suggest.rs` | Search suggestions, fetched on a background thread |
| `src/devtools.rs` | Finds tabs on the loopback DevTools port for the docked DevTools pane |
| `src/thumbs.rs` | Tab snapshots for hover cards, over the same DevTools port |
| `src/history.rs`, `src/crypto.rs` | Search history, encrypted on disk |
| `src/ui/newtab.html` | New-tab page, including the first-run setup screen (`#setup`, shown while `setup_done` is false) |

## Native UI: moving to the ANVL SDK

The native UI is being moved from our own `tiny-skia` + `cosmic-text` + `softbuffer` drawing code to the
**ANVL SDK** (`anvl_gpu` renderer on wgpu, `anvil_ui` element tree, version 0.3.0). **This is a direction,
not done yet: nothing in `src/` uses the SDK today, and the SDK is not in this repository.** Do not claim
otherwise in the UI, README or PR text.

Before writing any ANVL code, read the SDK's `ANVIL_AGENT_GUIDE.md`, then `ANVIL-UI-2.md` and `ADL-V3.md`
for the model and the design tokens. Its rule applies here too: if a widget is not listed under "Built",
it does not exist; build it from `div` and `canvas` and say so in the PR.

How the current pieces map (so new code can be written in the SDK's shape now):

| Today | In the SDK |
|---|---|
| `gfx::Canvas` shapes, `gfx::Icon` vector icons | `div()` styling, `icon(Icon::X)` (Lucide atlas); custom shapes inside a `canvas(...)` closure |
| `gfx::Fonts` (Segoe UI from `%WINDIR%`) | Inter / Noto Sans / Fira Mono compiled into `anvl_gpu` (no system font scan) |
| `gfx::Theme`, `PersonaColor` | `Theme` tokens (`Tok::Bg Card Primary ...`); persona colors become the primary token |
| `toolbar.rs` buttons, address bar, tab strip | `toolbar`, `icon_button`, `text_input`; the tab strip (drag to reorder, animated close) has no SDK component and stays custom, in a `canvas` |
| `popups.rs` menu, suggestions, tooltip | `menu`, `list_item`, `tooltip`. The hover card with a page snapshot is custom (`img` + `uniform_list`) |
| `ChromeCmd` out of the toolbar | one `Msg` enum per screen: `.on_click(Msg::X)`, then `ui.take_actions::<Msg>()` and `update` |
| `Toolbar::tick` returns the next wake time | `ui.next_wake()`; only request another frame while it is `Some` |

Rules for the move:

1. **Spike first, then port one surface at a time.** The open question is how `anvl_gpu::Renderer` draws into
   our `tao` window (the SDK's own runner owns a `winit` window and a UI thread) while WebView2 child windows
   sit on top of it. Until a spike answers that, keep `tiny-skia`, and do not add the SDK to `Cargo.toml`.
2. **Keep every item in "Things that look wrong but are deliberate".** In particular, presenting only the
   toolbar strip, parked non-focusable popups and the no-idle-CPU rule must still hold on the GPU path.
3. **Keep the IPC trust boundary.** `ChromeCmd` still comes only from native UI, and menu picks only from
   native popups; an SDK `Msg` is never built from web content.
4. **Follow the SDK's own rules:** unique stable ids on list rows, no `unwrap()`/`expect()` on runtime
   paths (`Renderer::new` returns a `Result`), idle apps sleep, and a headless test per screen.
5. **Say what you could not test.** The SDK needs a GPU backend (DX12 or WARP on Windows); agents without one
   can only run its headless tests.

## What must keep working

- Frameless window with tabs in the title bar and custom minimize / maximize / close buttons.
- Tab strip: animated open and close, drag to reorder, hover cards with page snapshots, right-click menu.
- A pre-loaded hidden tab so Ctrl+T is instant; background tabs suspended and set to low memory.
- Address bar with search suggestions (arrow keys + click), padlock, and `search.cederic.com` shown
  instead of `search.brave.com`.
- Right-click menu on pages (open link in new tab, copy, search selection, view source, inspect).
- DevTools docked on the right (F12), with a draggable splitter.
- Search history dropdown (clock button, Ctrl+H) and encrypted history file.
- Brave Search shown as **Cederic Search**: no Brave logos, name, or favicon visible (see `BRAND_SCRIPT`).

## Things that look wrong but are deliberate

- **The toolbar is painted straight onto the main window, and only the toolbar strip (plus the gap
  beside the DevTools pane) is ever presented** (`present_with_damage` in `Browser::redraw`). Pages are
  child windows on top, and the main window does not clip its children, so painting the whole window
  would flicker over the pages. Don't present the full buffer.
- **The event loop sleeps when nothing moves.** `Toolbar::tick` says when the next frame is needed
  (animations, hover-card timers, caret blink) and returns `None` when idle; keep it that way so the
  browser uses no CPU while you read a page.
- **Tabs are shown before the old one is hidden or closed** (`switch_to`, `open_tab`, `close_tab`).
  Reversing the order makes the bare window flash between tabs.

- **Every WebView must use the exact same `BROWSER_ARGS`, scroll bar style and data folder.** WebView2
  refuses to create a WebView whose environment options differ (error `0x8007139F`). If you change one,
  change them all.
- **Popups are never hidden, only parked off-screen** (`PARKED`, in *physical* pixels), and are
  non-focusable. tao can only show a window without stealing focus at creation time, and a focused
  window gets a blue accent border. Don't add `set_visible(true)` calls for popups, and don't park them
  with a logical position: at 200% scaling it overflows and Windows puts the window at the top-left.
- **Animations are paced by `pacer.rs`, not by the event loop's timeout.** Windows' default timer ticks
  every ~15.6ms, so `ControlFlow::WaitUntil(now + 16ms)` really waits ~31ms (30fps). The pacer thread
  uses high-resolution sleeps while something animates and is parked otherwise.
- **`devtools::get` reads exactly `Content-Length` bytes.** The DevTools HTTP server keeps the connection
  open; waiting for it to close times out and breaks DevTools docking.
- **`--remote-debugging-port=0`** makes the engine pick a free loopback-only port, written to
  `%LOCALAPPDATA%\Cederic\EBWebView\DevToolsActivePort`. Never change it to a fixed port or bind it to
  anything other than loopback.
- **Each tab's page has `window.__cedericTab = <id>`.** That is how `thumbs.rs` finds a tab's target.

## Security rules

- **Web content is untrusted.** The content IPC channel only accepts the fixed `Shortcut` strings and a
  validated `ContextRequest`. Never let content pages trigger anything else, and validate any new field.
- `ChromeCmd` comes only from the native toolbar, and menu picks only from the native popups.
- Don't add `unsafe` code; it is forbidden by the lint in `Cargo.toml`.
- Don't hand-write new cryptography. Use a well-reviewed crate. (`src/crypto.rs` is hand-written; replacing
  it with the `chacha20poly1305` and `sha2` crates is a welcome change.)

## Testing without taking over someone's computer

Don't send synthetic mouse or keyboard input to the desktop, and don't take screenshots of the screen.
The person may be using their computer, and you will click into their other apps. Instead, drive a
running Cederic through its debug port:

```
CEDERIC_EVAL="document.title" cargo test --release live_eval -- --ignored --nocapture
```

This runs the JavaScript in every page (popups, tabs) and prints the results. Dispatch DOM events
from that JavaScript to simulate clicks or hovers inside Cederic's own pages.

The native toolbar and popups can be rendered off-screen to PNGs, with no window at all, and a
toolbar frame can be timed:

```
CEDERIC_SHOT=<folder> cargo test --release render_ -- --ignored
cargo test --release bench_frame -- --ignored --nocapture
```

To run a test instance without touching the user's own browser profile, point `LOCALAPPDATA` at a
scratch folder for both the browser and `live_eval`.
