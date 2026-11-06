# Agent-to-Agent Communication & Auto-Sync (`INSTRUCTIONS.md`)

Two people and two AI agents work on this repository in parallel through GitHub.
**Every agent MUST read and update this file automatically on every task** so both agents stay in sync, know what files the other agent is touching, and combine features cleanly without overwriting each other's work.

 Also follow all rules in [AGENTS.md](AGENTS.md).

---

## 1. Automatic Workflow (Run Without Being Asked)

### Step A — Before Writing Any Code (Start of Task)
Run these commands automatically at the beginning of a task:

```powershell
git fetch --all --prune
git status
git branch -r
```

1. **Check active remote branches:** For every remote branch other than `origin/main` (e.g. `origin/<partner>/<topic>`), inspect what the other agent is currently doing:
   ```powershell
   git log origin/main..origin/<other-branch> --oneline
   git diff --stat origin/main...origin/<other-branch>
   git show origin/<other-branch>:INSTRUCTIONS.md
   ```
2. **Avoid collisions:** If the other agent's active branch is heavily modifying a section of `src/main.rs` or `src/toolbar.rs`, keep your edits in those files minimal and well-isolated so merging is clean.
3. **Create a fresh branch from `origin/main`:**
   ```powershell
   git switch -c <agent-or-user>/<short-topic> origin/main
   ```
4. **Post your claim below:** Add a row to [Active Work (In Progress)](#active-work-in-progress) in this file, commit, and push the branch right away (`git push -u origin HEAD`) so the other agent can see what you are working on via `git fetch --all`.

---

### Step B — Combining & Finishing (End of Task)
When your feature is done—or when the user says to combine/sync work—run this sequence automatically:

1. **Fetch the latest remote state:**
   ```powershell
   git fetch --all --prune
   ```
2. **Merge the latest `origin/main` into your feature branch:**
   ```powershell
   git merge origin/main
   ```
   - **If there are code conflicts:** Read the other agent's entries in [Completed & Handoff Log](#completed--handoff-log) and `git log origin/main -n 5` to understand their feature. Combine both changes so **both** features work together. Never delete the other agent's feature to resolve a conflict.
   - **If there is a conflict in `INSTRUCTIONS.md`:** Keep both agents' entries (never delete the other agent's log rows or notes).
3. **Move your entry from Active Work to Completed & Handoff Log:**
   - Remove your completed branch from [Active Work (In Progress)](#active-work-in-progress).
   - Append a new entry at the top of [Completed & Handoff Log](#completed--handoff-log) listing:
     - Branch / PR name
     - Summary of the feature or fix
     - Files and key structs/enums/functions changed (e.g. new `ChromeCmd` variants, new `Toolbar` fields, new IPC messages)
     - Anything the other agent must preserve when merging
4. **Verify build and tests (mandatory before committing):**
   ```powershell
   cargo build --release
   cargo test
   ```
   Both must succeed with **0 errors and 0 warnings**.
5. **Commit, push, and open/update the Pull Request:**
   ```powershell
   git push -u origin HEAD
   gh pr create --fill
   ```
   *(If a PR already exists for the branch, pushing updates it automatically.)*

---

### Step C — Pulling in the Other Agent's Unmerged Branch (When Asked to Combine Early)
If the human asks to combine your work with the other agent's branch before theirs is merged to `main`:

```powershell
git fetch --all --prune
git merge origin/<other-agent-branch>
```
Resolve any overlaps by preserving both features, verify with `cargo build --release` and `cargo test`, and record the merge in [Completed & Handoff Log](#completed--handoff-log).

---

## 2. Shared Agent Board

### Active Work (In Progress)
*Add a row here when starting a branch and push immediately so the other agent sees it via `git show origin/<branch>:INSTRUCTIONS.md`. Remove the row when moving it to the Completed log.*

| Agent / Owner | Branch | Files Being Touched | Task Summary |
|---|---|---|---|
| *(none currently)* | — | — | — |

---

### Messages & Coordination Notes Between Agents
*Leave persistent warnings, API contract changes, or questions for the other agent here.*

- **Shared hot spots (`src/main.rs`, `src/toolbar.rs`, `src/popups.rs`):** When adding new `ChromeCmd` variants, `Shortcut` variants, or `Toolbar` fields, append them at the end of the enum/struct rather than reordering existing lines. This prevents unnecessary Git merge conflicts when both agents add commands in parallel.
- **WebView2 options:** Remember that every WebView in `src/main.rs` must share identical `BROWSER_ARGS`, scrollbar style, and data directory (`0x8007139F` otherwise). If you change one, document it clearly below.

---

### Completed & Handoff Log
*Newest entries at the top. Append your entry here before opening a PR so the next agent merging `origin/main` knows exactly what changed.*

- **`claude/friendly-ptolemy-dzfqha` (Persona 2.0, Phase 0 / WS-A: `PageEngine` seam, no behavior change)**:
  - Plan: `docs/ENGINE_2_0_PLAN.md` (Servo as the base engine, WebView2 kept as fallback). This entry is the first step.
  - New `src/engine.rs` (engine-neutral `PageEngine`, `PageView`, `ViewConfig`, `ViewHandlers`, `Bounds`, `MemLevel`, `LoadPhase`, `Perm`, `Role`, `Source`) and `src/engine_webview2.rs` (`WryEngine`, the only wry user). `BROWSER_ARGS` and `USER_AGENT` moved there; `create_view` applies them to every view.
  - `main.rs`: `Tab.view` / `Devtools.view` / `account_probe` hold `Box<dyn PageView>`; `Browser.context: WebContext` became `Browser.engine: Box<dyn PageEngine>`; `build_tab`, `probe_google_account` and the DevTools frontend call `create_view`; `UserEvent::Load` carries `LoadPhase`; `main` returns `Result<(), Box<dyn Error>>`. Renamed calls: `set_memory_level`, `set_background`, `eval`, `eval_with_result` (takes a `Box<dyn Fn(String) + Send>`), `clear_browsing_data`.
  - Checked with `cargo check --target x86_64-pc-windows-gnu` (also `--tests` and `--release`): no errors or warnings. **Not run:** `cargo build --release`, `cargo test` and the app itself, because this was written in a Linux container with no Windows toolchain or Wine. Run both on Windows before merging, and open a tab, switch persona, F12 and Ctrl+T once.
  - Preserve when merging: new engine calls go through the traits, not wry; append new `ViewHandlers`/`PageView` members at the end. Still using CDP (next, WS-B): `devtools.rs`, `thumbs.rs`, `live_eval` test.

- **`claude/kind-brown-esznx2` (combined branch, auto-update, v0.2.0)**:
  - Merged `origin/gemini/picture-in-picture` (7df9d2d, site permissions) with the Google-accounts work. All four conflicts were additive and both sides were kept (`MenuAction`, `Browser` fields + init, tests).
  - **Auto-update** (`src/update.rs`): about 15s after start, reads `api.github.com/repos/plasmix2/PersonaBrowser/releases`, picks the highest version above `CARGO_PKG_VERSION` with an `.exe` asset from this repo's release URLs, and downloads it (size, `MZ` and GitHub's `sha256:` digest checked with the `sha2` crate). It renames the running exe to `*.old.exe` and puts the new one in its place. `update::cleanup()` removes the old file at start. `UserEvent::UpdateReady`, `Browser::{update_ready, exe_path}`, `MenuAction::RestartToUpdate`, and `Model::update_ready` (green dot on the persona button) are new. Off for exes under a `target` folder or with `PERSONA_NO_UPDATE=1`.
  - **Releases**: `.github/workflows/release.yml` builds, tests and publishes `Persona-vX.Y.Z.exe` when a `vX.Y.Z` tag is pushed (the tag must match `Cargo.toml`). Version **0.2.0**; release tag `v0.2.0-pre` published "Persona V0.2.0" as a pre-release.
  - The GitHub repo is now `plasmix2/PersonaBrowser` (the old name redirects).

- **`claude/kind-brown-esznx2` (passkey crash)**: the account probe is no longer built straight from a page-load event. `schedule_account_probe` sends `UserEvent::ProbeAccount` after a delay, and skips sign-in/challenge pages. Building a WebView runs a nested message loop that can collide with Windows' modal Windows Hello (passkey) dialog. Release builds now append panics (message + file:line) to `%LOCALAPPDATA%\Persona\crash.log` (hook in `main`). Ask users for that file when something crashes.

- **`claude/kind-brown-esznx2` (Google accounts per persona)**:
  - Each persona has its own WebView2 profile (`PersonaProfile::profile_name`, wry `with_profile_name`), so its own cookies and Google sign-in. `cederic` keeps the default profile, so earlier sign-ins stay with it. All WebViews still share `BROWSER_ARGS`, scroll bar style and data folder. Switching persona reopens all tabs in the new profile (`Browser::reopen_tabs_in_profile`).
  - The signed-in Google account is read by a hidden, throwaway WebView (`probe_google_account`). It opens `accounts.google.com/robots.txt` and POSTs synchronously to `ListAccounts` (the endpoint Chrome's avatar uses; GET returns 400), reading the text back via `evaluate_script_with_callback`. The page gets no IPC, its navigation is pinned to that one URL, and `parse_google_accounts` keeps only validated fields and googleusercontent.com photos. It runs at startup, on persona switch, and after any accounts.google.com page finishes loading.
  - The persona button shows the Google photo, then the persona picture, then the initial. The persona menu starts with "Sign in to Google", or name/email + "Manage your Google Account" / "Sign out of Google". New `MenuAction::OpenTab`, `UserEvent::{AccountProbeLoaded, GoogleAccount}`. Tests: `parses_google_account_list`, `persona_profiles_are_separate`.

- **`claude/kind-brown-esznx2` (persona avatar)**: the persona button (`Hit::Theme`) shows the active persona's profile picture cropped to a circle (`Toolbar::draw_avatar`, `bake_avatar`), or its initial on the persona color. New `Model::pfp` (appended), loaded through the favicon worker. `favicons::decode` now handles JPEG via the `jpeg-decoder` crate (the New Tab page saves pictures as JPEG). Tests: `decodes_jpeg`, and a `toolbar-pfp` case in `render_toolbar`.

- **`claude/kind-brown-esznx2` (saved sign-ins, popup speed)**:
  - Cookies are **no longer wiped** on exit or by "Clear search history" (`ChromeCmd::ClearHistory` now only clears the encrypted search history). That wipe signed users out of Google on every restart. Search history still auto-clears per session. Clearing cookies is now an explicit history-menu item: `MenuAction::ClearSiteData` → `ChromeCmd::ClearSiteData` ("Clear cookies and site data — Signs you out").
  - `popups::compose` caches the shadow pixmap and clip mask (`Popup::frame`, keyed by size/scale/theme). A popup frame went from 13.7 ms to 1.85 ms at 2x (`bench_popup`, ignored test), pixel-identical output.
  - Persona ids are unique even when created in the same millisecond (`Prefs::create_persona`). The persona test failed ~1 in 3 runs before.

- **`claude/kind-brown-esznx2` (rounded popups)**: popups are rounded and drop a soft shadow, drawn by us rather than by the OS. Each popup window is created with `with_transparent(true)` (tao enables DWM blur-behind, so the alpha we write is honored) and has a transparent margin (`popups::margins`). `popups::compose` draws a layered shadow, clips the card (rendered by `draw` into `Popup::card`) to a rounded rect, and strokes a 1px border. Buffer pixels are premultiplied `0xAARRGGBB`. `Popups::show` still takes the *card* position and size; the margin is added inside. `PERSONA_FLAT_POPUPS=1` restores opaque square popups. Keep `.with_undecorated_shadow(false)`.

- **`claude/kind-brown-esznx2` (popup rendering fix)**: the black edges, white corner and stale content around every popup came from tao's *default* `decoration_shadow: true` on undecorated windows. Its frame reserves unpainted strips around the client area. Popups now set `.with_undecorated_shadow(false)` explicitly (just removing the `true` call does nothing). Popups also paint synchronously in `Popups::show` and on animation frames in `Popups::tick`, instead of relying on `request_redraw` (redraws were being dropped, leaving a popup drawn only at fade 0). Tooltips are back to a native popup (`Kind::Tip`, `Content::Tip`, `Popups::tip_width`), and the in-page `__cedericTip` script is gone. Verified with `live_popups` (ignored test) under Wine + Xvfb.

- **`claude/kind-brown-esznx2` (tooltips)** — Toolbar tooltips are no longer popup windows. `Browser::show_tip` runs `window.__cedericTip(...)` (defined in `SHORTCUT_SCRIPT`) in the active page, which draws a click-through tooltip in a closed shadow root just under the toolbar. No popup window, nothing to resize or repaint. `ChromeCmd::Tip`/`TipEnd` and the toolbar hover timer are unchanged. Also restored the speed flags: `--enable-quic`, `EncryptedClientHello`, `UseEcoQoSForBackgroundProcess`, `--ssl-version-min=tls1.2`, `--disable-component-update`.

- **`claude/kind-brown-esznx2` (merge)** — Merged `origin/gemini/picture-in-picture` (4adecac) into the tooltip/popup work. Version is now `0.2.0` for the `persona` crate.
  - **`origin/main` (2dbe2b0) does not compile**: its merge left `BROWSER_ARGS` defined twice, duplicated blocks in `main.rs` (`sync_ui`, `redraw`), `gfx.rs` (glyph loop, `blend`), `popups.rs`, `thumbs.rs` (missing `EvalGesture`/`eval_gesture`), plus undefined `HistoryEntry`. `origin/gemini/picture-in-picture` does compile; use that as the base.
  - That branch's `BROWSER_ARGS` had dropped `--remote-debugging-port=0`, which DevTools docking, tab thumbnails and PiP depend on (see AGENTS.md). I restored it. It also no longer has `--enable-quic`, `EncryptedClientHello`, `--ssl-version-min=tls1.2` or the other perf flags from `agy/perf-improvements`; I left that to the owners to decide.
  - Popups no longer use `with_undecorated_shadow` (tao reserved unpainted insets, causing black/white edge artifacts); they draw their own 1px border and stay parked until repainted after a resize.

- **`claude/kind-brown-esznx2`** — Native tooltips on toolbar and window buttons (not built or run: written in a Linux container without the Windows/WebView2 toolchain, so `cargo build`/`cargo test` are still to be run on Windows):
  - **Files changed:** `src/toolbar.rs`, `src/popups.rs`, `src/main.rs`, `INSTRUCTIONS.md`.
  - **Key changes:** `ChromeCmd::Tip { text, x, y }` / `TipEnd` (appended), `popups::Content::Tip(String)` + `TIP_H`, `Toolbar::{tip_text, tip_rect, update_tip, end_tip}` with a 450ms `TIP_DELAY`, `Browser::show_tip`. Tooltips reuse the `Kind::Preview` popup window, so no new popup window was added. `Toolbar::tick` wakes for the tooltip timer and stays idle otherwise.

- **`agy/anonymous-history`** — Anonymous session search history & automatic clearing:
  - **Files changed:** `src/main.rs`, `src/history.rs`, `src/crypto.rs`, `INSTRUCTIONS.md`.
  - **Key changes:**
    - Removed OS username (`USERNAME`), machine hostname (`COMPUTERNAME`), and process ID (`std::process::id()`) from `crypto::derive_vault_key` and `crypto::random_bytes_32`.
    - Updated `HistoryStore::clear()` to rotate `history.key` with a fresh random seed and reset `next_id` before overwriting `history.enc`.
    - Auto-clears search history on startup and on browser close (`browser.quit`), and clears browsing data across all open tabs on `ChromeCmd::ClearHistory` and on exit.
    - Updated `Browser::history_menu` to show newest session searches first with an `"Anonymous session (Auto-clears)"` header and top `"Clear search history"` action, and added `<meta name="referrer" content="no-referrer">` + `rel="noreferrer noopener"` on search results in `BRAND_SCRIPT`.

- **`agy/newtab-autotype`** — Instant auto-typing in the address bar on New Tab open and focus lock:
  - **Files changed:** `src/main.rs`, `src/toolbar.rs`, `INSTRUCTIONS.md`.
  - **Key changes:**
    - Stopped calling `tab.view.focus()` when `tab.is_newtab` is true in `Browser::show_active()` (which was queuing an async WebView2 `MoveFocus` that stole Win32 keyboard focus back from the parent window right after `focus_address()`).
    - Added `Browser::ensure_chrome_focus()` and called it on `focus_address`, `ChromeCmd::FocusChrome`, `UserEvent::Prewarm`, `NewTabCmd::Ready`, `UserEvent::Load` (for active newtab), and `WindowEvent::Focused(true)`.
    - Added `NewTabCmd::AutoKey { key }` in `SHORTCUT_SCRIPT` and `Toolbar::auto_key(&mut self, key: &str)` so any printable key, `Backspace`, or `Enter` pressed on a New Tab (when not inside an `<input>`) is immediately fed into the address bar and transfers Win32 focus.

- **`agy/perf-improvements`** — Browser-wide UI, rendering, and site-loading performance overhaul (also integrated `f4194fe` from `gemini/picture-in-picture`):
  - **Files changed:** `src/main.rs`, `src/toolbar.rs`, `src/gfx.rs`, `src/popups.rs`, `src/thumbs.rs`, `src/input.rs`, `src/history.rs`, `src/ui/newtab.html`, `INSTRUCTIONS.md`.
  - **Key improvements & changes:**
    - **Site-loading & network acceleration (`BROWSER_ARGS`, `SHORTCUT_SCRIPT`, `BRAND_SCRIPT`, `newtab.html`, `input.rs`, `history.rs`):**
      - Removed `--disable-background-networking` (which disabled Chromium's `LoadingPredictor` and `PreconnectManager`), broken `DnsOverHttps<DoHTrial` fieldtrial params, `--dns-prefetch-disable`, `PostQuantumKyber`, and `StrictOriginIsolation`.
      - Enabled `NetworkServiceInProcess2,AsyncDns,HappyEyeballsV3,EncryptedClientHello,BackForwardCache,ParallelDownloading,CanvasOopRasterization,UseEcoQoSForBackgroundProcess` plus `--enable-quic --enable-gpu-rasterization --enable-zero-copy`.
      - Added `window.__cedericPreconnect` and Chromium `<script type="speculationrules">` (`prefetch` on moderate hover + `prerender` on conservative pointerdown) in `SHORTCUT_SCRIPT`, plus live address-bar preconnect while typing (`Browser::query`), New Tab preconnect for `search.brave.com` and bookmarks, and automatic preconnect for top search results on `search.brave.com`.
      - Prioritized `tab.view.load_url` before history disk I/O in `navigate_active`, eliminated redundant disk writes on deduplicated history records, and added zero-allocation fast-paths to `input::upgrade_to_https` and `input::is_allowed_navigation`.
    - **UI & rendering optimizations (`gfx.rs`, `toolbar.rs`, `popups.rs`, `thumbs.rs`, `main.rs`):**
      - Cached `fontdb::Database` and all 21 vector `Icon` paths in `OnceLock`s; cached search badge `Buffer` on `Toolbar`; replaced `hover_fade: HashMap` with `[f32; 16]`.
      - Cached `can_back`/`can_forward`, `window_title`, and `surface_size`; restricted `Browser::layout()` to only resize the active tab (plus open DevTools and any background PiP tab); coalesced thumbnail requests in `thumbs.rs`.
      - Integrated `f4194fe` (address bar Win32 focus transfer on `Hit::Omni` / `Ctrl+L`, `focusin` dismiss listener, incremental `MutationObserver` in `BRAND_SCRIPT`).
  - **What to preserve when merging:** Keep identical `BROWSER_ARGS` across all WebViews, keep `window.__cedericPreconnect` in `SHORTCUT_SCRIPT`, and keep lazy background-tab resizing in `Browser::layout` / `Browser::show_active`.

- **`gemini/picture-in-picture`** — Merged `origin/main` (native Rust toolbar/popups via `tiny-skia` + `cosmic-text` + `softbuffer`) and combined with Picture-in-Picture mode, Saved Bookmarks, and Theme/Persona switching (`Cederic` Orange default / `Uzaki` Slate Blue):
  - **Files changed:** `src/main.rs`, `src/toolbar.rs`, `src/gfx.rs`, `src/popups.rs`, `src/thumbs.rs`, `src/ui/newtab.html`, `README.md`, `INSTRUCTIONS.md`.
  - **Key structs / enums / functions added or updated:**
    - `gfx::Brand` (`Cederic`, `Uzaki`), `Theme::for_brand(dark, brand)` (with `card: Rgba`), and icons `Icon::Star`, `Icon::StarFilled`, `Icon::Bookmarks`, `Icon::Pip`, `Icon::Palette`.
    - `ChromeCmd` appended variants: `Pip`, `ToggleBookmark`, `BookmarksMenu { x, y }`, `ThemeMenu { x, y }`.
    - `Shortcut` appended variants: `Pip` (`Alt+P`), `Bookmark` (`Ctrl+D`), `Bookmarks` (`Ctrl+B`).
    - `MenuAction` appended variants: `ToggleBookmark`, `DeleteBookmark(String)`, `SetBrand(Brand)`.
    - `NewTabCmd` variants: `OpenBookmark { url }`, `DeleteBookmark { url }`, `SetBrand { brand }`.
    - `Prefs` & `Bookmark` persisted to `%LOCALAPPDATA%\Cederic\prefs.json`.
    - `thumbs.rs`: event-driven `rx.recv()` worker (no periodic `captureScreenshot` white-flash loop) combining `Pixmap` thumbnails with CDP `Runtime.evaluate(userGesture: true)` for `EvalGesture` (`Alt+P` / toolbar PiP).
  - **What to preserve when merging:** Keep background tab PiP bounds (`rect(-10.0, -10.0, 1.0, 1.0)`) in `Browser::hide_tab` and `Browser::layout` so PiP video continues decoding when switching tabs; keep `Theme::for_brand` and `Prefs` sync across `toolbar`, `popups`, and `newtab.html`.
- **`agy/agent-instructions`** — Added `INSTRUCTIONS.md` and wired `AGENTS.md`, `CLAUDE.md`, and `GEMINI.md` to reference it so both agents automatically sync via `git fetch --all`, communicate active branches/files, and merge `origin/main` cleanly.
