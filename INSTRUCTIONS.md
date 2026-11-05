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
- **ANVL SDK:** the native UI draws through `src/anvil.rs` (see `AGENTS.md`). New native UI should use `Painter`; write new screens as message-in, state-out (`Msg` enum plus an `update`) so they can become `anvil_ui` element trees later. Claim any `vendor/anvil` change in the Active Work table first.
- **WebView2 options:** Remember that every WebView in `src/main.rs` must share identical `BROWSER_ARGS`, scrollbar style, and data directory (`0x8007139F` otherwise). If you change one, document it clearly below.

---

### Completed & Handoff Log
*Newest entries at the top. Append your entry here before opening a PR so the next agent merging `origin/main` knows exactly what changed.*

- **`claude/sharp-sagan-263h83` (rendering bug sweep)**: popups can now be rendered headless with `CEDERIC_SHOT=<dir> cargo test render_popups -- --ignored` (every popup kind, dark and light, over a light page). Fixed from those renders: the orb shader vanished on light cards (it added light; it now blends the glow color by strength), the shuffle chip was invisible in light mode, the hue track had notches at its rounded ends (now square segments between round caps), popup shadows were too dark and heavy, a menu row's label was cut to a few letters when it had a long hint (the hint is dropped instead; the Google email moved into its submenu), the Site permissions note was cut off, the confirmation dialog had a large gap. Toolbar: a narrow active tab drew its icon and close button on top of each other (it now shows only the centred close button; `close_rect` matches), the setup veil was applied twice over part of the toolbar (now two non-overlapping regions). The scrim crops the page snapshot instead of stretching it. New checks in `anvil::tests::gpu_frames` (all GPU checks share ONE engine in ONE test: two devices created in parallel tests crashed WARP on CI with an access violation): a WGSL error in the vendored shader blanks every frame, so this guards it.

- **`claude/sharp-sagan-263h83` (persona editor freeze fix)**: `Popups::tick` runs after every event of the event loop; with the persona editor's shader animating it repainted (GPU frame + read-back) on every mouse move and key press, starving input. It now draws at most one animation frame per `next_anim` (33 ms while the shader runs, `pacer::FRAME` otherwise). `UserEvent::Frame` only repaints the toolbar when the toolbar has a frame due (`Browser::next_frame`), so popup animations no longer also repaint the toolbar 60 times a second.

- **`claude/sharp-sagan-263h83` (persona menu freeze fix)**: the submenu popup's transparent shadow margin used to overlap its parent menu and swallow clicks on the parent's rows. `Kind::Sub` now has no left margin (flush to the parent; margins 0/6/20/32) and `show_sub` places the window exactly at the parent's edge (or 20 px further left when flipping). `Browser::sub_for` stops hover from re-showing an already open submenu. Vendor patch: `render_to_frame` keeps off-screen textures and read-back buffers per size (up to 8) because the toolbar and each popup alternate sizes every frame. Rule of thumb: any popup window's transparent margin takes mouse input, so never let one overlap another popup or the toolbar.

- **`claude/sharp-sagan-263h83` (expressive persona editor, custom colors)**: `PersonaColor::Custom(0xRRGGBB)` (stored in `PersonaProfile::color` as `#rrggbb`; `to_key`, `hex_string`, `from_str_name` parses it; `accents`/`surface_tints` are no longer `const fn`; `Theme::for_persona` derives a full palette from the accent via `gfx::{hsl, to_hsl}`). Persona editor: hue and vibrancy sliders (draggable rows: `Row::slider`, `Pick::Slide`, `slide_at`), a shuffle button (row 14, `Icon::Sparkles`), the orb flares on every change (`PersonaForm::pulse`, decays in `Popups::tick`), custom color name/hex under the dots. `ColorChoice` in `main.rs` (preset or custom hue/vibrancy). A dialog that updates itself must not repaint the scrim (`show_modal`), and `Popups::show` keeps `pressed` for `Kind::Modal` so drags survive refreshes. Row indices of the persona dialog: 0 name, 1..=8 presets, 9 delete, 10 cancel, 11 save, 12 hue, 13 vibrancy, 14 shuffle.

- **`claude/sharp-sagan-263h83` (native dialogs, simpler buttons, orb shader)**: removed the hover gradient (`ai_glow`, `rrect_gradient`, `Toolbar::shimmer`); toolbar buttons use a flat hover fade, the New Tab plus is a plain icon (no chip, no rotation). Native dialogs (no web modals): `Content::{Persona, Confirm}` in `popups.rs`, `ModalKind` / `PersonaFormState` / `Browser::{open_persona_editor, show_modal, modal_key, save_persona_form, ask_delete_persona}` in `main.rs`. The modal popup is not focusable, so the main window keeps the keyboard and `modal_key` / `ReceivedImeText` feed the persona name field; Esc closes. `Row` has `x`/`w` for rows narrower than the card (`row_hit`). New Tab page's persona New/Edit/Delete buttons send `edit_persona` / `ask_delete_persona` (`NewTabCmd::{EditPersona, AskDeletePersona}`); the old HTML persona and permission modals are hidden dead markup. The persona editor has a live shader preview: **vendor patch** adds `Gradient::Orb` / `RectStyle::orb` and a `kind > 3.5` branch in `shaders.wgsl` (animated glowing orb tinted by `fill2`), exposed as `Painter::orb`; `Popups::t0` supplies time and `Popups::tick` keeps redrawing while the editor is open. Picking a color previews it on the chrome (`preview_persona_color`, undone by `hide_modal`). Still web: the setup screen and the New Tab page itself (setup needs the keyboard, wallpaper picking needs a file input).

- **`claude/sharp-sagan-263h83` (smoother tabs, morphing selection)**: tab `x`/`w` now move with critically damped springs (`damp`, carries velocity, no snapping at the end) and tab edges are not pixel-snapped while anything moves (`Toolbar::px`, `moving`). The selected-tab look is ONE shape (`Toolbar::sel`, `spring`, slight overshoot) that morphs from tab to tab instead of per-tab fades (`TabVis::act` now only drives text color / hover). Drag blur strength follows the real distance to the dragged tab and is eased per tab (`TabVis::blur`).

- **`claude/sharp-sagan-263h83` (new-tab flicker, drag blur)**: the next spare WebView is built 500 ms after a tab opens (it blocked the UI thread right as the tab animated in); a brand-new active tab starts fully "selected" (`TabVis::act`) and 60% open. While a tab is dragged, the other tabs get a progressive backdrop blur by slot distance (`Painter::blur_region`, layer 1) and the dragged tab is drawn on layer 2 above it (`Painter::layer`). Layers: 0 base, 1 blurs/veils, 2 on top.

- **`claude/sharp-sagan-263h83` (memory, click robustness, setup every launch, Google compat)**: `anvil::Engine` is now a cheap clonable handle on ONE GPU engine shared by the toolbar and all popups (was two devices); `main` creates it and passes clones to `Toolbar::new_with_brand` / `Popups::new`. Vendor patch in `vendor/anvil/crates/anvl_gpu/src/render.rs`: `render_to_frame` keeps its off-screen texture and read-back buffer (`offscreen`, `readback`) and the adapter is `LowPower`. Mouse moves only repaint when something changed (`Toolbar::take_dirty`), popups find the pressed row from the last pointer position (`Popup::cursor`). Setup runs once per `SETUP_VERSION` (now 2) for every profile, saved data or not (`PrefsData::setup_version`); while it runs the page behind is blurred by CSS and the native chrome is blurred and locked (`Toolbar::set_locked`, `Painter::veil`; only window buttons and the title bar work). Ctrl+Shift+T reopens the last closed tab (`ChromeCmd::ReopenTab`, `Browser::closed_tabs`). `BROWSER_ARGS` (identical for all WebViews): removed `UserAgentClientHint` from the disabled features, `CanvasOopRasterization` and `--enable-zero-copy` (Google Docs).

- **`claude/sharp-sagan-263h83` (submenus, menu icons/avatars, tab motion)**: `MenuEntry`/`MenuItem` gained `icon`, `avatar` (`popups::Avatar`: photo URL or letter on color) and `sub`; new `Kind::Sub` popup, `Pick::{Sub, Hover}`, `Browser::{show_sub, menu_subs, sub_actions}`, `Popups::{row_anchor, image_loaded, has_image}` (menu photos are fetched by the favicon worker and forwarded from `UserEvent::IconLoaded`). Persona dropdown: Google account row (photo, submenu: manage / sign out), **Personas** submenu (avatars, New/Manage), background, Site permissions. The linked Google photo becomes the active persona's picture while it has none (`set_google_account`). Page context menu has icons plus Copy page address and Print (`MenuAction::Print`). 14 more Tabler icons. Tabs: the New Tab plus is a chip aligned with the tab row and opens on mouse **press**; new tabs start part-open and animate faster; the active look cross-fades (`TabVis::act`); a dragged tab lifts with a shadow and settles on drop (`Toolbar::lift`, `lift_id`).

- **`claude/sharp-sagan-263h83` (mandatory setup, Google link, no test personas, grouped buttons)**: fresh profiles start with one persona (`PersonaProfile::new_main`, id `main`, own WebView2 profile `persona_main`) instead of Cederic + Uzaki. `migrate_test_personas` (skipped under `cfg!(test)`, which keeps the two-persona defaults the unit tests rely on) drops an untouched Uzaki and, when only an untouched "Cederic" is left, turns `setup_done` off so setup runs. Setup (newtab.html `#setup`) is 4 steps: name + color (name required), **link Google** (required: `NewTabCmd::{LinkGoogle, CheckGoogle}`, `Prefs::google_hint`, `NewTabState::google`; the account probe result now calls `sync_all_newtabs`), permissions, done. There is no Skip, and `NewTabCmd::FinishSetup` is ignored until `Browser::google` is set. While setup is not done a start-up URL argument is ignored (only the New Tab page opens). Toolbar right side is grouped `[star bookmarks history] | [pip devtools] | persona` (`GROUP_GAP`, dividers); the persona button is now rightmost.

- **`claude/sharp-sagan-263h83` (animation/UX pass)**: `Painter::icon_rot` (rotate an icon about its centre). Toolbar: buttons dip to 86% while pressed (`press_scale`), the New Tab plus turns a quarter on hover, reload spins into stop while loading, the star pops when a page is bookmarked (`star_pop`), the address bar focus ring fades and grows (`focus_f`), Ctrl+1..8 / Ctrl+9 switch tabs (when the chrome has focus). Popups keep hover fades when their content is refreshed (no flash when a switch is toggled).

- **`claude/sharp-sagan-263h83` (popup responsiveness + modal blur)**: popup windows' transparent shadow margins are part of the window and take clicks, so margins are smaller again (20/6/20/32) and `Kind::Tip`/`Kind::Preview` windows now `set_ignore_cursor_events(true)` (never clicked). Frame pacing is 60 fps (`pacer::FRAME`) because every chrome frame is a GPU draw plus read-back; popup entrance is a fade only (no slide, 0.12s). New `Kind::Scrim` popup (`Content::Scrim`, created before `Kind::Modal` so it sits below it): a box-blurred (`anvil::blur_pixmap`) active-tab thumbnail dimmed over the page while the Site permissions dialog is open; `Browser::{show_scrim, hide_modal}`. The scrim swallows clicks. Use `hide_modal`, not `popups.hide(Modal)`, to close the dialog.

- **`claude/sharp-sagan-263h83` (Tabler icons + Gemini-style hover glow)**: all chrome icons are now Tabler Icons (MIT, `src/tabler.rs`, license in `src/ui/TABLER-LICENSE.txt`), parsed from SVG path data by `anvil::parse_path` into GPU strokes; the atlas special-casing is gone. Button hover uses `Painter::ai_glow` (blue/violet/pink wash pulled 35% toward the persona accent, turning slowly). To add an icon: add the enum variant, `ALL_ICONS` entry and a `tabler::paths` arm with the `d` strings from tabler.io/icons.

- **`claude/sharp-sagan-263h83` (ANVL polish: icons, hover gradient, shadows, menu)**: translucent strokes were showing every segment overlap, so `Painter::stroke` now pre-blends against `Painter::backdrop` and draws opaque (set `c.backdrop` when drawing over a different color). Icons ANVL's atlas has (back, forward, plus, x, search, info, code) use the atlas. Toolbar buttons get a slowly turning accent gradient on hover (`Toolbar::shimmer`, `Painter::rrect_gradient`). Popup shadows are ANVL soft shadows (`Engine::shadow_pixmap`, used by `popups::compose`; margins grew). The persona menu lost the three permission toggles, color hints and the "Persona" hint; `MenuAction::{ToggleCamera,ToggleNotifications,ToggleMicrophone}` were removed (permissions live in the native Site permissions dialog).

- **`claude/sharp-sagan-263h83` (native UI now runs on the ANVL SDK)** (type-checked for `x86_64-pc-windows-msvc` from Linux with `cargo check --target x86_64-pc-windows-msvc --tests`: 0 warnings. **Not run on Windows**: `cargo test` and the real window are still to be tried there. `Toolbar` and `anvil.rs`/`gfx.rs` were rendered headlessly on lavapipe in a scratch harness and looked right):
  - **Vendored** `vendor/anvil` (SDK 0.3.0: `anvl_gpu`, `anvil_ui`, docs). New deps `anvil_ui` (path) and `glam`; `cosmic-text` removed.
  - **`src/anvil.rs` (new):** `Engine` (headless renderer + `Ui`), `Painter`, `Weight`, chrome icon drawings. Chrome is rendered off-screen and copied into the existing softbuffer strip, so main-window presentation, focus and input are untouched.
  - **`src/toolbar.rs`:** painting is `Toolbar::paint(&mut Painter)`; the address bar is `buf: String` + `anvil_ui::Editor` (was a cosmic-text `Editor`); fields `fonts`, `pretty`, `badge`, `placeholder`, `avatar*`, `TabVis::title` removed; `engine`, `bg_uploaded` added. Tab titles now ellipsize instead of fading. Hit-testing, `tick`, `ChromeCmd`s unchanged.
  - **`src/popups.rs`:** `Row` holds strings (`tail` for the bold part of a suggestion); `Popups::engine` replaces `fonts`; `View::shot_ready` for the tab snapshot upload.
  - **`src/gfx.rs`:** now only colors, themes, `Icon` names and `rrect_path`. `Canvas`, `Fonts`, the coverage curve and `warm_fonts` are gone (ANVL ships its fonts, so there is no system font scan to hide). `Icon::Palette` removed (unused).
  - **Preserve when merging:** all drawing goes through `Painter`; do not reintroduce `tiny-skia` text. The lazy background loading and `setup_done` from the earlier entry below still apply.

- **`claude/sharp-sagan-263h83` (faster start, setup screen, cleaner text/icons, ANVL SDK direction)** (written in a Linux container without the Windows/WebView2 toolchain: `cargo build`/`cargo test` for the full crate are **still to be run on Windows**. `gfx.rs` was compiled and its new tests run in a scratch crate; `newtab.html` was rendered in Chromium):
  - **Files changed:** `src/main.rs`, `src/gfx.rs`, `src/ui/newtab.html`, `AGENTS.md`, `INSTRUCTIONS.md`.
  - **(Superseded by the ANVL entry above: `warm_fonts` and the coverage curve no longer exist.)**
  - **Loading:** `gfx::warm_fonts` (new) reads the UI font database on a thread started first thing in `main`, so the multi-MB CJK font reads overlap window/WebView creation (`gfx::font_db` is the shared `OnceLock`). `Prefs::open` now reads only the *active* persona's background (`read_background`); others load in `Prefs::ensure_background`, called from `set_active_persona` and `delete_persona`. Anything new that reads `background_for`/`bg_strip_for` for a non-active persona must call `ensure_background` first (`prefs_per_persona_custom_background` does). `navigate_active` starts `load_url` before `close_suggest`/history/repaint. No `BROWSER_ARGS` changes.
  - **Setup screen:** first-run overlay in `newtab.html` (`#setup`, `window.__setupSync`): name + color for the active persona (`update_persona`), site permissions (`set_permissions`), shortcuts. New `NewTabCmd::FinishSetup` (appended), `PrefsData::setup_done: Option<bool>` and `Prefs::{setup_done, finish_setup}`, `NewTabState::setup_done`. Prefs files from before this change that already have personas count as set up. Note: camera, microphone and notifications default to *allowed* (`SitePermissions::default`); the screen says so rather than hiding it.
  - **Text and icons (`gfx.rs`):** glyph coverage goes through a small power curve (`coverage_lut`, light vs dark text by luma) in `Canvas::text` and `blend_px`; `Canvas::icon` snaps its box to whole physical pixels. Tests: `text_tests::*`, `setup_runs_once_per_version_even_with_saved_data`, `other_personas_backgrounds_load_when_selected`.
  - **ANVL SDK:** `AGENTS.md` now has "Native UI: moving to the ANVL SDK" (mapping, spike-first plan, rules). Nothing uses the SDK yet and it is not vendored; the first step is a spike on drawing `anvl_gpu` into the `tao` window under the WebView2 children.
  - **Preserve when merging:** the `warm_fonts` thread at the top of `main`, the lazy background loading, and `setup_done` in `Prefs::save`.

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
