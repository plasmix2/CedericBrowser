# Anvil Design Language (ADL) v3

> [!IMPORTANT]
> **SUPERSEDED:** All ADL specifications have been consolidated into the unified [ADL.md](file:///c:/Users/venne/Documents/anvil_engine/ADL.md) (v3.0). Refer to `ADL.md` for the authoritative design language contract.

ADL v3 is about how Anvil **moves and responds**. v2.1 defined what things look like at rest; v3 defines what happens between states: motion tokens, real elevation, feedback that can be undone, and UX rules every screen follows.

ADL v3 extends `ADL.md` (v2.1) and `ADL-MOBILE.md`. Every token, type role and component there still applies unless this document overrides it. Section numbers match v2.1 where a section is changed.

The reference implementation is `crates/anvil_ui`. ANVL Note (`apps/anvil_note`) is the first app built on v3.

---

## 1. What changed from v2.1

| Area | v2.1 | v3 |
|---|---|---|
| Motion | One exponential rate per widget, picked per call site | Named duration tokens, curves, and springs in `theme::motion` (section 4) |
| Enter and exit | Not modelled. Things appeared and vanished in one frame | `Ui::presence`: timed enter and exit with their own durations (4.2) |
| Lists | Rows snapped into place | Rows rise in, staggered, and glide when the order changes (4.3) |
| Elevation | Shadow tokens existed, renderer did not draw them | Renderer draws soft shadows. Every Layer 1 surface has one (3.3) |
| Opacity | Each widget faded its own colors | One frame-wide opacity (`Ui::set_opacity`) fades a whole subtree (4.4) |
| Layers | 2 | 3: base, floating, and top (toasts and tooltips) (3.4) |
| Toasts | One toast, fade only, no actions | A stack with one action, hover to hold, swipe or X to dismiss, a countdown line (8.10) |
| Tooltips | None | Delayed hover tooltips on icon-only and shortcut-bearing controls (8.11) |
| Errors | Helper text only | Helper text plus a shake (4.5) |
| Destructive actions | Act, then say what happened | Act at once and offer Undo (section 11) |
| Type | Inter 400/500, Noto Sans 400 | Adds Inter SemiBold and italics, Noto Sans Medium, and a mono role (Fira Mono) (section 2) |
| Scrolling | Followed the finger, stopped dead on release, hard stop at the ends | Fling with friction, rubber-band ends (4.9) |
| Glass | Tokens only, drawn opaque | Real backdrop blur under every floating surface, with an off switch (3.5) |
| Touch text editing | Tap to place the caret only | Long press and double tap select words, draggable handles, a Cut/Copy/Paste menu (ADL-MOBILE 9.1) |

---

## 2. Typography (extended)

Eight faces, all OFL, all compiled into the binary (`crates/anvil_ui/assets/fonts/README.md`).

| Role | Family | Faces | Line height | Use |
|---|---|---|---|---|
| `Ui` | Inter | Regular 400, Medium 500, SemiBold 600, Italic 400, Medium Italic 500 | 1.25 | Labels, controls, headings, numbers |
| `Body` | Noto Sans | Regular 400, Medium 500 | 1.5 | Paragraphs, and strong text inside them |
| `Mono` | Fira Mono | Medium 500 (its only weight) | 1.5 | Code, file names, figures that must line up |

`TextStyle` builders: `.medium()`, `.semibold()`, `.body()`, `.mono()`, `.italic()`.

### 2.1 Weight rules (changes v2.1 section 2.1)

| Weight | Where |
|---|---|
| Regular 400 | Body copy, descriptions, placeholders, input values |
| Medium 500 | Headings up to 20 px, buttons, tabs, badges, all-caps labels, bold inside paragraphs |
| SemiBold 600 | **Display sizes only** (22 px and up): the screen title, the one headline number. Never on controls |

Bold (700) still does not exist. A heavier look comes from size, not weight.

### 2.2 Italic and mono

- Italic is for emphasis and quotes. Body text has no italic face of its own, so italic in any role is drawn with Inter Italic. The engine does this; callers just ask for `.italic()`.
- Mono is for content that is code or must align: code blocks, inline code, paths, hashes, tabular figures. Never for UI labels.
- A role asks only for weights it has (`FontRole::weight_for`). Mono always draws Medium. Body caps at Medium. Without this, cosmic-text treats a missing weight as a miss and falls back to a different family.

### 2.3 Scale additions

| Name | Size | Face | Use |
|---|---|---|---|
| Display | 56 | Inter SemiBold | One headline number |
| Screen title | 26 | Inter SemiBold | One per phone screen (ADL-MOBILE section 7) |
| Code | 0.9 × the surrounding text | Fira Mono | Code inside the editor and in content |

---

## 3. Shape and depth

### 3.3 Elevation (changed)

Layer 1 surfaces now draw `SHADOW_OVERLAY` for real. The renderer feathers a rounded rectangle over a *softness* width (`RectStyle::soft(px)`, shader lane `params.z`), centered on the edge like a CSS blur.

```
0 8px 24px rgba(0, 0, 0, 0.45)
0 2px 6px  rgba(0, 0, 0, 0.30)
```

- `Ui::shadow(rect, radius)` draws both shadows. Draw it before the surface, on the same layer.
- `Ui::surface(rect)` is a complete Layer 1 card: shadow, `CARD` fill, 1 px `BORDER`, `LG` radius.
- Dropdown menus, floating panels, modals, toasts and tooltips all use it.
- Layer 0 still never has a shadow. Shadows never show focus or selection.

### 3.4 Draw layers (new)

| Layer | Holds | Pointer |
|---|---|---|
| 0 | Base UI | Normal |
| 1 | Menus, popovers, sheets, modals, drop hints | Blocks what is under it (`pointer_overlay`, `pointer_modal`) |
| 2 | Toasts and tooltips | Toasts block under their cards. Tooltips never take the pointer |

Within a layer all shapes draw before all text, so two surfaces that overlap must sit on different layers.

---

### 3.5 Glass and backdrop blur (new)

Floating surfaces are glass: the layers beneath them show through, blurred, under an 80% tint. It tells the eye the surface floats without hiding where it came from.

| Part | Rule |
|---|---|
| Who gets it | Every Layer 1 and top-layer surface: menus, popovers, modals, sheets, toasts, tooltips, the touch selection menu. Never Layer 0 |
| Tint | The surface's own fill (`CARD`, or `CARD_HI` for tooltips and the touch menu) at `GLASS_ALPHA` 0.8, the v2.1 `CARD_GLASS` |
| Blur | Dual Kawase: the scene beneath, downsampled twice to a quarter size and upsampled once to half. About 12 px of blur at 1x |
| Edge | 1 px `BORDER`, as every floating surface. The shadow still draws |
| Text on glass | `TEXT` or `TEXT_DIM` (`MUTED` in the ANVL tokens), never `DISABLED_TEXT` for information |
| Fading | A glass surface's blur fades with it (the frame opacity applies) |
| Off | `Ui::set_backdrop_blur(false)`: glass draws its tint opaque, so every surface reads the same without the effect. Use it for a reduce-transparency setting or a slow GPU. ANVL Note: `ANVIL_NO_BLUR=1` |

API: `Ui::glass(rect, radius, tint)` draws shadow, blur window and tint in one call; `Ui::surface(rect)` is glass with `CARD` and `LG`. `Ui::backdrop_blur(rect, radius)` is the bare window, for custom surfaces.

**How it renders.** A frame with no glass draws in one pass straight to the screen, as before. When a layer holds glass, the layers beneath it draw to an offscreen texture; three small passes blur it; it is copied to the screen; then each glass area shows the blurred copy through a rounded mask before its layer's shapes draw. Glass on layer 2 over glass on layer 1 sees the same snapshot (the layers beneath the *first* glass layer), so a tooltip over a menu blurs the page, not the menu.

## 4. Motion (rewritten)

### 4.1 Tokens

`anvil_ui::motion`. Use a token; do not invent a duration.

| Token | Seconds | Use |
|---|---|---|
| `INSTANT` | 0.09 | Hover, press, color changes |
| `FAST` | 0.16 | Tooltips, small fades, focus, toast close button |
| `BASE` | 0.24 | Toasts, menus, list items entering, rows gliding |
| `SCREEN` | 0.28 | Screen-to-screen transitions |
| `SLOW` | 0.32 | Sheets, modals, error shake |
| `STAGGER` | 0.04 | Delay between list items entering, for at most `STAGGER_MAX` (5) items |
| `TOOLTIP_DELAY` | 0.5 | Rest time before a tooltip shows |
| `TOAST_TTL` / `TOAST_ACTION_TTL` | 3.2 / 6.0 | How long a toast stays when nothing holds it |

`motion::rate(d)` converts a duration into the rate `Ui::animate` takes (`3 / d`, about 95% settled).

| Curve | Use |
|---|---|
| `ease_out_cubic` | Entrances and moves |
| `ease_in_cubic` | Exits |
| `SPRING` (260, 23) | About 4% overshoot. Indicators, sheets, releases |
| `SPRING_CALM` (300, 35) | No overshoot. Stacks that must not bounce, such as toasts |

### 4.2 Presence

`Ui::presence(id, visible, enter, exit) -> f32` returns 0 (hidden) to 1 (shown). Use it for anything that appears and disappears.

- A presence starts hidden, so the first frame it is visible it animates in.
- Reversing midway continues from where it is. Nothing jumps.
- `Ui::set_presence(id, visible)` puts it at an end state, for example to replay an entrance.
- Map the value to opacity and a small offset (8 to 24 px). Never to size.

### 4.3 Lists

- **Entrance.** `Ui::entrance(id, index)` plays once per id: wait `STAGGER × min(index, 5)`, then fade in and rise 12 px over `BASE`. Call it for every row, including rows scrolled out of view, so they are finished when they arrive.
- **Reorder.** A row's position is an animated value keyed by the row's id, so when the order changes (an edited note moves to the top, a search filters the list) rows glide to their new place over `BASE`.
- **Removal.** A removed row disappears at once; the toast that offers Undo is the feedback.

### 4.4 Opacity

`Ui::set_opacity(a)` multiplies the alpha of every shape, icon, image and text pushed until it is set back. It returns the previous value so fades nest: `let prev = ui.set_opacity(ui.opacity() * t); ...; ui.set_opacity(prev);`. It resets to 1 every frame.

### 4.5 Error shake

`Ui::shake(id)` starts it, `Ui::shake_offset(id)` reads the horizontal offset: 4 px left and right, three times, dying away over `SLOW`. Shake the control that rejected the input, never the screen. The shake always comes with a reason (helper text, a toast, or an empty state).

### 4.6 Screen transitions

On Compact, the incoming screen slides 24 px in from the side it comes from and fades in over `SCREEN` (`ease_out_cubic`). Forward (list to item) comes from the right; back comes from the left. Headers and bars do not move.

### 4.7 Reduced motion

Every helper in this section honors `Ui::set_reduced_motion`: presences, entrances, springs and toasts jump to their end state, and shakes return 0. Information shown by motion must also be shown by color, text or icon.

### 4.9 Scrolling physics (new)

Every scroll area (`Ui::scroll_begin`) and the text editor share one model (`ui::coast`):

| Phase | Behavior |
|---|---|
| Drag | Content follows the finger 1:1. Speed is measured as a smoothed px/s |
| Past an end | Content moves at `RUBBER_BAND` (0.55) of the finger, up to 30% of the view |
| Release (touch) | The measured speed carries on and decays at `FLING_FRICTION` (3.2 /s). Under 20 pt/s it stops |
| Fling reaches an end | It stops at the end. No bounce |
| Release past an end | Eases back to the edge at 14 /s |
| Touch during a fling | Catches the content: it stops where it is, and the touch is not a tap |
| Mouse wheel, mouse drag | No momentum and no overscroll: wheels already have their own |
| Reduced motion | No fling, no rubber band |

### 4.8 Power

Motion requests frames with `Ui::request_wake` only while something is moving. A settled screen asks for no frames. A toast asks for 30 frames a second for its countdown line, and none once it is gone.

---

## 8. Components (additions)

### 8.10 Toast stack

`Toasts` holds the stack; `Ui::toast_stack(&mut toasts, area) -> Option<u64>` draws it once per frame, last.

| Part | Rule |
|---|---|
| Place | Bottom of `area`, above anything pinned there. Right-aligned, 360 px wide on Medium and Expanded. Full width minus 16 px margins on Compact |
| Card | Layer 2, `surface` style, 68 px tall, a 3 px status-color accent bar on the left |
| Text | Title 15 Medium `TEXT`, message 14 `TEXT_DIM`, both ellipsized |
| Action | At most one, a Secondary button at the right (for example Undo). Clicking it returns the toast's handle and dismisses it |
| Countdown | A 1 px line along the bottom edge in the status color at 50%, shrinking as time runs out |
| Stack | Newest at the bottom, 8 px apart, at most 3. A fourth pushes the oldest out. Positions glide on `SPRING_CALM` |
| Enter / exit | Rise 16 px and fade in over `BASE`. Fall 8 px and fade out over 0.2 s |
| Hold | Hovering a toast pauses its countdown and shows its close X |
| Dismiss | X, swipe right past 35% of its width, or timeout. A shorter swipe springs back |
| Repeats | Pushing the same title and message as the top toast restarts it instead of stacking a copy |

Routine success (a save that the status bar already shows) should not need a toast. Use toasts for results the user cannot see otherwise, and for anything that can be undone.

### 8.11 Tooltip

`Ui::tooltip(id, anchor, text)`, called right after the control with the control's id.

- Shows after the pointer rests on the anchor for `TOOLTIP_DELAY`. Hides on press or leave.
- Below the anchor, or above when it would leave the window. Clamped 8 px inside the window.
- `CARD_HI` fill, 1 px border, `MD` radius, shadow, 13 px Medium `TEXT`, 8 × 5 px padding. Layer 2.
- Fades and slides 4 px over `FAST`.
- Touch input never shows a tooltip. A tooltip only repeats what the icon means or names a shortcut; it never holds information found nowhere else.
- Every icon-only button has one. Controls with a keyboard shortcut name it: `New note  (Ctrl+N)`.

### 8.12 Search field

A `text_field` with the Search icon and a "Search …" placeholder, at the top of the list it filters.

- Filters as you type. Every word must match (title or body, case-insensitive).
- Enter opens the first match. With no match, the field shakes.
- Escape clears the query.
- The list's count label shows `n of total` while a query is active.
- `Ctrl+F` focuses it from anywhere. On Compact it first returns to the list.

### 8.13 Empty state

When a list has nothing to show it says why and what to do: a 28 px muted icon, a 15 px Medium title, and a 13 px hint, centered, fading in over `BASE`. Never leave a blank pane.

### 8.14 Markdown in the editor

The note editor styles Markdown as you type (`anvil_ui::markdown`). It styles; it does not render. Every character stays on screen, so the caret, selection, search and undo work on exactly what was typed. Markup characters recede to `DISABLED_TEXT` so the text reads as formatted.

| Source | Look |
|---|---|
| `# ` to `###### ` | Inter Medium, `TEXT`, at 1.6×, 1.35×, 1.15×, then 1× the body size. The row grows to fit. `#`s at the same size in `DISABLED_TEXT` |
| `- `, `* `, `+ `, `1. `, `1) ` | Marker in `MUTED`. Leading spaces allowed, so nested lists work |
| `- [ ] ` / `- [x] ` | Task. A ticked task's text drops to `DISABLED_TEXT` |
| `> ` | 3 px `BORDER` bar beside the line, text in `MUTED` italic |
| ```` ``` ```` block | `SURFACE` panel with a 1 px border and `MD` radius behind the whole block, text in `CODE` (Fira Mono, 0.9×). No Markdown inside |
| `` `code` `` | `CARD_HI` pill behind, text in `CODE` |
| `**bold**` | Noto Sans Medium, `TEXT` |
| `*italic*` | Inter Italic |
| `---`, `***`, `___` | A 1 px `BORDER` line across the column |

- `CODE` is `#d4d4d8`. Code is set apart by its panel, not by hue (v2.1 section 1.2).
- Span colors are fixed when the text is shaped, so they carry the frame opacity (quantized to 1/32) and fade with screen transitions.
- Rich text is a general engine feature: `TextSystem::layout_rich` takes sorted, non-overlapping `RichSpan`s that may change the font, weight, size, color and slant of a byte range.

### 8.15 Undo and redo

Every multi-line editor keeps its own history (`EditorCursor::history`).

- **Ctrl+Z** undoes, **Ctrl+Y** or **Ctrl+Shift+Z** redoes. The caret and selection come back with the text.
- **Grouping.** A run of typing is one step, and so is a run of deleting. A space, a pause of `UNDO_GROUP_GAP` (1 s), a caret move, or a different kind of edit starts a new step, so undo takes back about a word at a time. Paste, cut, Enter, Tab and Shift+Tab are always steps of their own.
- An edit that changes nothing (Backspace at the start) leaves no step.
- A new edit after an undo clears redo.
- At most `UNDO_LIMIT` (200) steps per note. History lives for the session and is not saved.
- Undo is an edit: it marks the note dirty and syncs like typing.

---

## 11. UX rules (new)

1. **Undo over confirm.** A destructive action happens at once and offers Undo in a toast for `TOAST_ACTION_TTL`. Ask first only when the action cannot be undone at all.
2. **Every action answers.** A click, key or drop produces a visible change within one frame: a state change, a toast, a shake, or an empty state.
3. **Keyboard reaches everything.** Every frequent action has a shortcut, and the shortcut is named in a tooltip or hint where the action lives.
4. **Motion explains, never decorates.** Animate where something came from, where it went, or that it changed. Nothing loops except busy and countdown indicators.
5. **Never block on motion.** Input is live during every animation. A control mid-transition is already clickable at its drawn position.
6. **Never lose the user's place.** Transitions, search, and window resizes keep focus, caret, scroll and the open item.

### 11.1 ANVL Note shortcuts

| Keys | Action |
|---|---|
| Ctrl+N | New note |
| Ctrl+S | Save and sync now |
| Ctrl+Z | Undo |
| Ctrl+Y / Ctrl+Shift+Z | Redo |
| Ctrl+F | Search notes |
| Alt+↑ / Alt+↓ | Previous / next note in the (filtered) list |
| Enter (in search) | Open the first match |
| Esc | Clear search; on Compact, back to the list |
| Tab / Shift+Tab | Move focus |

---

## 12. Implementation status

| Item | Status |
|---|---|
| Motion tokens, curves, springs (`theme::motion`) | Built, tested |
| Soft shadows in the renderer, `Ui::shadow`, `Ui::surface` | Built. Menus, floating panels, modals, toasts, tooltips use it |
| Frame-wide opacity | Built, tested |
| Third draw layer | Built |
| Presence, staggered entrance, error shake | Built, tested |
| Toast stack with action, hold, swipe, countdown | Built, tested. Used by ANVL Note |
| Tooltip | Built. Used in ANVL Note header and sidebar |
| ANVL Note: Compact screen slide, row entrance and glide, search, empty state, undo delete, Alt+arrows | Built |
| Backdrop blur behind Layer 1 and top surfaces (`Ui::glass`, on/off switch) | Built. Runs without GPU errors; judge the look by eye |
| Sheet with drag-to-dismiss (ADL-MOBILE 8.4) | Not built |
| Command palette in ANVL Note | Not built (exists in the component showcase) |
| Note row context menu (pin, duplicate, rename) | Not built |
| Rich text shaping (`TextSystem::layout_rich`, `RichSpan`) | Built |
| Markdown styling in the editor (headings, lists, tasks, quotes, code, emphasis, rules) | Built, tested |
| Editor undo and redo with word grouping | Built, tested |
| Undo in single-line text fields | Not built |
| Inter SemiBold and italics, Noto Sans Medium, Fira Mono; `FontRole::Mono` | Built, tested |
| Fling and rubber-band scrolling (scroll areas and the editor) | Built, tested |
| Touch selection: long press, double tap, handles, Cut/Copy/Paste/Select all menu | Built, tested |
| Double click selects a word; drag after it extends by words (desktop) | Built, tested |
| Android clipboard (`ClipboardManager` through JNI) | Built. Not yet run on a device |
| Bottom navigation bar, phone editor toolbar, swipe-to-delete rows | Built. See ADL-MOBILE v2 |
| ANVL Note private sync: a random pairing code per install, Pair a device sheet | Built, tested end to end (two installs pair through the relay; a third does not receive) |
| ANVL_GPU crate (`crates/anvl_gpu`): renderer, own glyph atlas, clip stack, gradients, strokes and arcs, headless rendering | Built, pixel-tested on the GPU |
| Own PNG, JPEG (baseline), GIF, BMP decoders; baked icon atlas; own logo rasterizer | Built, tested against Pillow and resvg |
| Video through the OS decoder (Media Foundation with DXVA, MediaCodec) | Windows: played three real files on the hardware decoder. Android: compiles, not yet run on a phone |
| Progressive JPEG, WebP | Not supported yet |
| Links, tables and images in Markdown | Not styled |
| Showcase page for v3 motion and components | Not built |
| Web tokens (`anvl-web/adl-web`) for v3 motion | Not updated |

## 13. Checklist (additions to v2.1 section 12)

1. Pick every duration and curve from `motion`. If none fits, change this document first.
2. Anything that appears or disappears uses `presence` or the toast stack.
3. Floating surfaces use `surface` or `shadow`, on layer 1 or 2.
4. Destructive actions offer Undo.
5. Icon-only controls have a tooltip; shortcuts are named.
6. Every list has an empty state.
7. Check reduced motion: the screen must read the same with every animation at its end state.
