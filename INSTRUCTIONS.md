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

- **`gemini/picture-in-picture`** — Implemented independent per-persona custom background images:
  - **Files changed:** `src/main.rs`, `src/ui/newtab.html`, `INSTRUCTIONS.md`.
  - **Key additions:**
    - Per-persona background storage (`background_cederic.txt`, `background_strip_cederic.png`, `background_uzaki.txt`, `background_strip_uzaki.png`) with automatic migration from legacy single-background files.
    - `Prefs::background_for(brand)` and `Prefs::bg_strip_for(brand)` to separate wallpaper and blurred toolbar strip per persona (`Cederic` vs `Uzaki`).
    - Switching persona instantly switches both the active newtab background wallpaper and the native toolbar background strip.
    - Updated Palette theme dropdown menu and New Tab page button/tooltip labels to dynamically indicate and control the active persona's background (`Cederic Background` vs `Uzaki Background`).
    - Added unit test `prefs_per_persona_custom_background` verifying independent storage, retrieval, persistence, and persona-switching isolation.

- **`gemini/picture-in-picture`** — Implemented borderless Full Screen mode (F11, Escape, right-click menu, and HTML5 video / YouTube fullscreen):
  - **Files changed:** `src/main.rs`, `src/toolbar.rs`, `INSTRUCTIONS.md`.
  - **Key additions:**
    - F11 shortcut support both in web views (`SHORTCUT_SCRIPT`) and toolbar (`Toolbar::key`), mapped to `ChromeCmd::ToggleFullscreen`.
    - Escape shortcut support (`Shortcut::Escape`) to exit fullscreen mode.
    - Right-click page context menu entry: "Full Screen" / "Exit Full Screen" with "F11" shortcut hint (`MenuAction::ToggleFullscreen`).
    - Full HTML5 Fullscreen API support (`fullscreenchange` and `webkitfullscreenchange` event listeners over IPC with `UserEvent::FullscreenState`).
    - Seamless borderless fullscreen toggle via `tao::window::Fullscreen::Borderless(None)`.
    - `top_bar_height(&self)` and `devtools_width(&self)` set to 0.0 in fullscreen, giving the active webview 100% display area without visible native chrome.

- **`gemini/picture-in-picture`** — Fixed Google Account sign-in ("This browser or app may not be secure") and YouTube 0:59 playback disconnection:
  - **Files changed:** `src/main.rs`, `src/thumbs.rs`, `src/ui/newtab.html`, `INSTRUCTIONS.md`.
  - **Key fixes:**
    - Removed `--remote-debugging-port=0` from `BROWSER_ARGS` (which forced Blink into automation mode, setting `navigator.webdriver = true` and tripping Google's botguard / account sign-in block).
    - Added `--disable-blink-features=AutomationControlled` to `BROWSER_ARGS`.
    - Added clean standard desktop Chrome `USER_AGENT` (`Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36`) via `with_user_agent` on `WebViewBuilder` to strip `Edg/...` and embedded WebView2 tokens.
    - Added `STEALTH_SCRIPT` deleting `Object.getPrototypeOf(navigator).webdriver` and hiding `window.chrome.webview` from web page detection while maintaining secure internal IPC routing via `window.__cedericSendIpc`.
    - Fixed YouTube 0:59 SABR playback interruption by avoiding CDP evaluation attachment on YouTube tabs, removing `__cederic*` globals from subframes, and preventing tab suspension.


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
