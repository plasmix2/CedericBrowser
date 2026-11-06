# anvil_ui 2.0: plan

A declarative element tree with flexbox layout, a theme in context, and real event handling, built
next to today's immediate-mode API and adopted by ANVL Note one screen at a time.

Status: **Phases 0, 1, 2 and 3 done** (see below). Decisions taken: actions (not entities), light theme in
Phase 3, kiosk demo frozen on a compatibility module, Taffy 0.14 with flexbox, grid and block.

---

## 1. Why change

What the code does today, and what it costs (see also `ADL-MISSING.md` sections 1, 4 and 5):

| Today | Cost |
|---|---|
| Every rectangle is computed by hand. ANVL Note's `app.rs` calls `ui.s(...)` 100 times | Screens are hard to change; nothing sizes to its content; wrapping rows (chips, tags) are impractical |
| Colors and radii are `const` (`palette::CARD`, `anvl::TEXT`), in two competing sets | No light theme, no brand themes, no high contrast; `SURFACE` means two different colors |
| Widget ids are `&'static str`; a list row needs `intern()`, which leaks a string per id | Every dynamic list leaks; ids collide silently when reused |
| Interaction is decided inside each widget call, against the rectangle it was given | Hover/press only work where a widget knows its rect up front; no bubbling, no "click outside", no keyboard order from structure |
| Note-specific widgets (`anvil_header`, `note_row`, `status_bar`) live in `anvil_ui/src/ui/anvl.rs` | The engine carries one app's UI |

What stays, because it is good: `anvl_gpu` (rendering, text, media), the ADL-V3 motion tokens and
springs, the text editor's editing model (`EditorCursor`, `History`, touch selection), scroll
physics, the toast stack logic, and the zero-idle frame loop.

---

## 2. The model

### 2.1 One frame

Like GPUI, the tree is rebuilt every frame from app state, then laid out, painted and hit-tested
in the same frame. Nothing is retained between frames except small per-element state (animations,
scroll offsets, focus), keyed by element id.

```
app.render(&state, cx) -> Element      build: cheap structs, no layout yet
  -> Taffy layout (text measured)        layout: every node gets a Rect
  -> paint into anvl_gpu::DrawList       paint: same DrawList, layers, clips as today
  -> hit-test this frame's input         events: deliver to the topmost target, bubble up
  -> app.update(action)                  actions run after the frame, with &mut state
```

Using this frame's layout for hit testing (not last frame's) means a click always lands on what
is drawn under the pointer, even mid-animation.

### 2.2 Actions instead of closures (**Decide**)

Rust will not let a tree borrowed from `&state` also hold closures that mutate `state`. GPUI
solves it with entity handles and a big runtime. Anvil can use a smaller, proven shape: elements
carry an **action value**, and the app handles actions after the frame (Elm / iced style).

```rust
enum Msg { NewNote, Open(NoteId), Delete(NoteId), Query(String) }

button("New note").primary().icon(Icon::Plus).on_click(Msg::NewNote)

// after the frame:
for msg in cx.take_actions() { app.update(msg) }
```

- Typed, testable (an action list is easy to assert on), no `Rc<RefCell<..>>`.
- Closures stay available for local view state: `.on_hover(|hovered, s: &mut RowState| ..)`
  where the state lives in the element-state store, not the app.
- Recommendation: actions. Alternative: GPUI-style entities, more flexible, roughly twice the work.

### 2.3 Ids

`ElementId` is a 64-bit hash built from a name and optional keys: `id("note").key(&note.id)`.
No leaks, no `'static` requirement. Elements without an explicit id get one from their position
in the tree (parent id + child index), which is stable as long as structure is stable; anything
in a dynamic list must use `.key(..)`. A debug build warns when two live elements share an id.

### 2.4 Coexistence

Both APIs draw into the same `DrawList` and share `Ui` state (focus, animations, scroll, toasts):

- `ui.element(rect, el)` lays out and paints a new-style tree inside an old-style rectangle.
- `canvas(|ui, rect| ...)` is a new-style element that hands its laid-out rectangle to old-style
  code (the text editor, custom paint).

So any screen can move independently, and nothing breaks between phases.

---

## 3. The API

### 3.1 Elements

| Element | What it is |
|---|---|
| `div()` | Container: flex or grid, style, children |
| `text("…")` | A text run or paragraph, measured by `TextSystem`, wraps to its width |
| `icon(Icon::X)` | An icon from the baked atlas |
| `img(texture)` | A picture with `Fit` |
| `canvas(f)` | Custom paint (and the bridge to old code) |
| `uniform_list(id, count, row_h, f)` | Virtualized list: only rows in view are built |
| `scroll(id)` | A div whose overflow scrolls, with today's fling and rubber band |

Plus components built from these (section 3.4).

### 3.2 Styling: Tailwind-like builders, ADL tokens

```rust
div()
    .flex_col().gap_4().p_4()
    .bg(cx.theme().card).rounded_lg().border_1().border_color(cx.theme().border)
    .child(
        div().flex_row().items_center().justify_between()
            .child(text("Battery").heading())
            .child(badge("94%").success()),
    )
    .child(button("Pair phone").primary().on_click(Msg::StartPairing))
```

- Spacing is the ADL scale in logical px: `gap_1` = 4 … `gap_4` = 16, `p_4` = 16.
- Sizes: `w(px)`, `w_full()`, `flex_1()` (ADL `Fill`), and no size at all is ADL `Hug`.
- State styles: `.hover(|s| s.bg(..))`, `.active(..)`, `.focus(..)`, `.disabled(..)`.
- Motion: `.transition(Motion::Fast)` animates color, opacity, offset and scale between states with
  the V3 tokens; `.enter(Enter::Rise)` uses `Ui::entrance`; `.spring()` for indicators.
- Radii, borders and shadows come from the theme: `.rounded_lg()`, `.border_1()`, `.glass()`.

### 3.3 Theme in context (**Decide**: ship light theme in Phase 0 or Phase 3)

```rust
pub struct Theme {
    pub name: &'static str,
    pub dark: bool,
    pub bg: Color, pub surface: Color, pub sunken: Color,
    pub card: Color, pub card_hi: Color, pub border: Color, pub ring: Color,
    pub text: Color, pub text_dim: Color, pub muted: Color, pub disabled: Color,
    pub primary: Color, pub primary_fg: Color, pub code: Color, pub selection: Color,
    pub success: Color, pub warning: Color, pub danger: Color,
    pub radius: Radii, pub glass_alpha: f32, pub font_scale: f32,
}
Theme::adl_dark()     // today's look, the default
Theme::adl_light()
Theme::high_contrast()
```

- `cx.theme()` everywhere; `ui.set_theme(..)` swaps it at runtime, and a theme change animates
  (colors ease over `FAST`).
- One palette: `palette` and `anvl` merge. `sunken` (#09090b, inputs) and `surface` (#121215,
  bars and sidebars) end the `SURFACE` conflict.
- Old-API widgets read the theme too (Phase 0), so a theme switch covers both APIs.

### 3.4 Components (in `anvil_ui`, ADL states built in)

button, icon_button, text_input, text_editor (wraps today's editor), checkbox, toggle, slider,
segmented, tabs, badge, tooltip, list_item, nav_bar, toolbar, menu / context menu, sheet, modal,
toast_stack, progress (bar and ring), skeleton, empty_state, search_field.

App-specific pieces leave the engine: `anvil_header`, `note_row`, `status_bar`, `LogoTextures`,
`SyncState` move into `apps/anvil_note` as that app's own components.

### 3.5 Events

| Event | Behavior |
|---|---|
| Pointer | Hit test topmost element (by layer, then paint order); hover enter/leave; press, release, click (release inside after press inside) |
| Bubbling | Target first, then ancestors. The nearest handler emits and the event stops there unless that element is `.bubble()` (decided in Phase 2: a delete button in a clickable row must not also open the row). `stop_propagation()` stops events at an element without handlers; a disabled element swallows them. `on_click_outside` for menus and sheets |
| Gestures | Tap, double tap, long press (500 ms), drag with axis lock (8 pt slop), swipe, fling: today's thresholds, moved into one recognizer |
| Scroll | Wheel and touch to the nearest scroll container under the pointer |
| Keyboard | Focus order follows the tree; Tab/Shift+Tab; Enter/Space activate; Escape bubbles to the nearest dismissable |
| Text | Typed text, paste, and (new) IME composition, delivered to the focused input |
| Hit boxes | Touch hit boxes grow to 48 dp on Android (ADL-MISSING 2.A) without changing the visual |

### 3.6 Layout

- Taffy 0.14 (flexbox, grid, block; about 3 new crates). A `TaffyTree` is reused across frames
  and cleared each frame, so a frame allocates almost nothing once warm.
- Text leaves measure through `TextSystem::layout` with the width Taffy offers; shaped buffers are
  cached by `TextKey` already, so measuring and painting share one shaping.
- Logical px in, physical px out: the root scales once, as today.
- Budget (revised after measuring in Phase 1): about 3 ms to build and lay out 1,200 elements,
  half of them text, in a release build on the development PC; Taffy itself is about half of
  that. The original 1 ms target was optimistic. A regression test holds the line at 6 ms.
  `uniform_list` (Phase 3) keeps long lists out of the count; caching unchanged subtrees across
  frames is the next step if real screens need more.

### 3.7 Runtime: threads and frame pacing

- `anvil_ui::runner` runs an app on its own UI thread. The window's event thread (the OS's
  main thread) only creates the window and forwards events. Windows blocks that thread while the
  title bar is held, dragged or resized; the UI thread keeps drawing (checked: 269 to 282 fps
  in ANVL Note while the window's thread sat in the move loop, `GUI_INMOVESIZE` set).
  Android's surface hand-over is synchronous: on suspend the event thread waits until the UI
  thread has let go.
- Frames are on demand: the UI thread sleeps on its channel until an event, a timer, or an
  animation asks for a frame (`Ui::request_frame`). Animations are paced by vsync, not by a
  60 Hz timer, so they run at the display's rate (279 Hz on the development PC). A minimized
  window, which presents nothing, backs off to a few checks a second.
- Live resize: on `Resized` the event thread waits (at most `RESIZE_WAIT`, 100 ms) until the UI
  thread has drawn a frame at the new size, so the window's edge and its content move together.
  Measured in ANVL Note by tracking the sidebar's 1 px border right after each of 40 size steps:
  without the wait, 22 of 40 frames were Windows stretching the old frame (worst 24 px off);
  with it, 1 of 40 (3 px). Each size step completes in about 12 ms. The swapchain keeps one
  frame in flight (`desired_maximum_frame_latency: 1`): a frame costs about a millisecond, so
  queueing a second only adds latency.
- The launcher's phone mirror is scrcpy's window, embedded as a child of another process. While
  the launcher's window thread sat in the title-bar move loop (all 10 samples), scrcpy's thread
  answered every `WM_NULL` ping within 1 ms, as before and after: the mirror does not stall
  during a drag.
- Workers wake the UI thread when they send (`Proxy`), so no app polls on a timer for them.
- Measured in ANVL Note (release, `ANVIL_NOTE_BENCH=1`): building the whole UI takes
  0.06 to 0.09 ms a frame, render (glyphs, uploads, encode, submit) 0.6 to 0.9 ms, and the rest
  of a frame is waiting for the display. Idle, nothing is drawn (`next_wake None`).
- Decided: no region caching or partial redraw for now. With the build at under 0.1 ms, caching
  regions could save at most that, at the cost of invalidation bugs. Revisit if a screen's
  build or render shows up in the bench above a millisecond or two.

---

## 4. Phases

Each phase ends green: everything compiles, all tests pass, ANVL Note works on desktop and phone.

### Phase 0: Foundations (about 1 week)

- `Theme` struct and `cx.theme()`; merge `palette` and `anvl`; every existing widget reads the
  theme. No visual change in dark mode (checked by headless snapshots).
- `ElementId` hashing; animation, scroll and focus stores keyed by it; `&'static str` keeps working
  through a conversion, and `intern()` is deleted from ANVL Note.
- Add Taffy. Headless snapshot test harness (`render_to_frame` + reference PNGs).

Exit: Note looks identical, `intern()` gone, theme switch works on today's widgets.

**Done.** What landed:
- `Theme` (`anvil_ui::Theme`, `Theme::adl_dark()`), on `Ui` as `ui.theme()` / `ui.set_theme()` and the
  public field `ui.theme`. All 232 token uses in `anvil_ui`'s widgets and ANVL Note's 60 read it.
  `palette` and `anvl` stay as constants only for the frozen kiosk demo; a test pins
  `adl_dark()` to their exact values.
- `ElementId` (`anvil_ui::ElementId`): FNV-1a hash, `.key(..)` and `.part(..)`. Every widget takes
  `impl Into<ElementId>`, so string literals still work. `intern()` and its leak are gone from
  ANVL Note (rows use `ElementId::new("anvl_note").key(&note.id)`).
- Taffy 0.14 (one new crate in the tree) with a test that lays out text measured by
  `TextSystem`, side by side and wrapped (`anvil_ui::flex`).
- Screenshot harness (`crates/anvil_ui/tests/snapshots.rs`, references in `tests/snapshots/`):
  core widgets, Note widgets, and glass. All three matched pixel for pixel before and after the
  theme and id changes. A fourth test proves a theme switch recolors today's widgets.
  Update references with `ANVIL_UPDATE_SNAPSHOTS=1 cargo test -p anvil_ui --test snapshots`.

### Phase 1: Element tree and layout (about 2 weeks)

- `div`, `text`, `icon`, `img`, `canvas`, style builders, Taffy layout, paint to `DrawList`.
- `ui.element(rect, el)` and `canvas` bridges.
- Snapshot tests: a card, a wrapping chip row, a hug-height paragraph, a fill/flex row, grid.

Exit: a static settings-style screen built only from elements renders identically on desktop and
headless.

**Done.** What landed (`crates/anvil_ui/src/element/`):
- `div()`, `text()`, `icon()`, `img()`, `canvas()`; children, ids.
- Builders: `flex_row/col`, `flex_wrap`, `grid_cols`, `col_span`, `items_*`, `justify_*`, `gap*`,
  `p*`/`m*` (ADL 4 px steps: `gap_4` = 16), `w`/`h`/`size`/`w_full`/`w_frac`/`min_*`/`max_*`,
  `flex_1` (ADL Fill), `grow`, `shrink_0`, `absolute` with `top/right/bottom/left`; `bg`,
  `border_1`, `border_color`, `rounded*`, `shadow`, `glass`, `opacity`, `overflow_hidden`;
  text: `heading`, `title`, `body`, `caption`, `label`, `medium`, `semibold`, `italic`, `mono`,
  `nowrap`, `color`, `text_size` (color and size inherit, as in CSS).
- Colors and radii take theme tokens (`Tok::Card`, `Radius::Lg`) resolved at paint, so a theme
  switch needs no rebuild.
- Taffy layout in physical px with text measured by `TextSystem` (max-content, min-content =
  widest word, wrapping); fixed sizes always win over measured content; Taffy rounds to whole
  pixels.
- Bridges: `ui.element(rect, tree)` and `ui.layout_element(rect, &tree)`; `canvas(|ui, r| ..)`
  runs immediate-mode code inside a tree.
- Tests: 9 layout tests (hug card, fixed + fill row, wrapping paragraph, wrapping chips, grid,
  absolute, inheritance, 2x scale, nowrap) and two GPU snapshots (`elements_settings`,
  `elements_bridges`), plus the cost guard above.

### Phase 2: Interaction (about 2 weeks)

- Hit testing, hover/active/focus styles, actions, bubbling, click-outside.
- Keyboard focus from tree order. `scroll` containers with today's physics.
- Gesture recognizer (tap, double tap, long press, drag, swipe) replacing per-widget copies.
- `.transition()` and `.enter()` on V3 motion tokens.

Exit: tests drive synthetic input through a tree and assert the actions it emits.

**Done.** What landed:
- One frame of `ui.element`: extract handlers, layout, then the interaction pass
  (`ui/elements.rs`: scroll containers, hit test, pointer, gesture and keyboard dispatch), then
  paint. Input acts on where elements are this frame.
- Actions: `.on_click(msg)`, `.on_double_click`, `.on_long_press`, `.on_click_outside`,
  `.on_escape`, and `.on_drag(|d| msg)` / `.on_swipe(|dir| msg)` mapping gesture data. Any
  `'static` value; the app reads them with `ui.take_actions::<Msg>()` after the frame, so trees
  without actions need no type parameter.
- Hit testing: topmost in paint order, inside every clip above it, through today's gates
  (immediate scroll areas, modals, popups); 48 dp touch boxes for anything with a pointer
  handler. Click = release over an element the press started on; a finger that dragged, a
  press that scrolled, or one a long-press or drag handler took never clicks. The `Pointer` and
  `NotAllowed` cursors follow.
- State styles `.hover(|s| ..)`, `.active(..)`, `.focus(..)` (bg, border color, text color,
  opacity; paint-only, so no relayout) blended by `.transition(motion::INSTANT)`;
  `.disabled(true)` greys a subtree and blocks its input.
- Keyboard: anything with `on_click` (or `.focusable(true)`) joins the Tab order in tree order,
  with the focus ring; Enter/Space click the focused element; Escape goes to the nearest
  `on_escape` around focus, or else the innermost one, once per frame.
- `.scroll()` containers on today's wheel, drag, fling and rubber-band physics (split out of
  `scroll_begin`); only the innermost container under the pointer takes input; content keeps its
  height; children are clipped for drawing and for the pointer.
- `anvil_ui::gesture::Recognizer`: tap, double tap (350 ms), long press (500 ms, with a wake so
  it fires on an idle app), drag with axis lock (8 pt slop), swipe (500 pt/s). `Ui` runs it once
  per frame (`ui.gestures()`); today's widgets keep their own copies until Phase 4 moves them.
- `.enter(Enter::Rise | Enter::Fade)` and `.enter_at(.., index)` on `Ui::entrance` (BASE,
  STAGGER).
- Fixed on the way: a childless root ignored its own size. Leaves now go through Taffy's
  `compute_leaf_layout`, so fixed sizes, min/max and padding apply to every leaf.
- Tests: 5 recognizer tests, 15 synthetic-input tests (`tests/interaction.rs`: click, bubbling,
  stop, disabled, click-outside, Tab order and activation, Escape, long press, double click,
  drag, swipe, wheel scrolling, nested containers, touch scroll vs. tap, 48 dp, hover cursor and
  idle wake) and the `elements_states` snapshot. A full frame of 1,200 interactive elements
  (layout, interaction, paint) is about 2.9 ms in release, guarded at 8 ms.
- Not yet: hover enter/leave actions, drag arbitration between `.on_drag` and an enclosing
  scroll container (both move today), `.layer(..)` for hit testing across separate trees, exit
  motion, and IME. They come with the components that need them in Phase 3.

### Phase 3: Components and showcase (about 2 to 3 weeks)

- The section 3.4 components, each with its ADL states, in both themes.
- `uniform_list`. Light and high-contrast themes.
- A showcase app in `apps/` listing every component in every state, both themes, both tiers.

Exit: every component has a snapshot test per state and theme.

**Done.** What landed:
- Themes: `Theme::adl_light()`, `Theme::high_contrast()` (black canvas, white 1 px borders, a
  yellow focus ring, opaque glass) and `Theme::all()`. `ui.switch_theme(to)` eases every color
  over `FAST` (`ui.set_theme` still swaps at once, `ui.target_theme()` says where a switch is
  heading). Glass now reads the theme's `glass_alpha`. Unit tests hold every text role to
  4.5:1 (7:1 for text in high contrast) on every surface, and the ring to 3:1.
- `uniform_list(id, count, row_h, |i| row)`: only the rows in view, plus 3 either side, are built
  (`ui.element` lowers the list to a scroll container with one spacer as tall as all rows, lays
  it out once to learn the viewport, fills in the rows and lays out again). A frame with a
  million rows costs about 1.1 ms in release (guarded at 8 ms by an ignored test).
- Components, in `anvil_ui::components`: `button` (primary, secondary, ghost, destructive,
  success), `icon_button`, `checkbox`, `toggle`, `slider`, `segmented`, `tabs`, `nav_bar`,
  `toolbar`, `list_item`, `menu`, `modal`, `sheet`, `tooltip`, `toast_stack`, `badge`,
  `progress` (bar and ring), `skeleton`, `empty_state`, and `text_input`, `search_field` and
  `text_editor`, which wrap today's text widgets so the editing core is unchanged. Each takes
  `.id`, `.disabled`, `.tooltip` and `.pin(..)`; controls emit the app's own actions.
- What the components needed from the element API: `.pin(Pin::Hover | Pressed | Focus)` (hold a
  state on), `.on_pointer(|p| ..)` (position inside the element, for sliders), `.on_key(key,
  action)`, `.slide(x)` (a spring-driven offset for knobs and indicators), `Paint::Tint` /
  `Tok::tint(a)` (theme colors at an opacity), negative and per-side margins, `Ui::emit`.
- The items Phase 2 left open: `.on_hover_enter(a)` / `.on_hover_leave(a)`;
  `.on_drag_axis(axis, f)` (a horizontal row in a vertical scroll container shares a touch by
  axis, and plain `.on_drag` now keeps a touch scroll container still); `.layer(n)` on a tree's
  root (draws on layer `n`, and blocks the pointer and Tab for lower trees under its root);
  `.presence(shown, Enter)` for exit motion (the element stays in the tree, fades and takes no
  input while leaving, and no space once gone); IME composition (`Input::preedit`, shown
  underlined at the caret of a text field, `Ui::ime_area()` for the candidate window, and
  `events::ImeState` / `WindowInput::sync_ime` to wire a window; ANVL Note, the starter and the
  showcase are wired. The multi-line editor receives committed text but does not draw the
  composition yet).
- Text now gets a pixel of slack in layout and in paint alike: Taffy rounds both edges of a
  box, which could take almost a pixel off a text box and wrap a line that fit (the
  `elements_settings` reference changed for that reason: a paragraph that fits now stays on one
  line).
- The showcase, `apps/anvil_showcase`: every component in every state, live, with a theme
  switch (dark, light, high contrast) and a tier switch (desktop grid, phone column; a narrow
  window starts on the phone tier). `ANVIL_SHOWCASE_SHOTS=<dir> cargo test -p anvil_showcase
  screenshots` writes six PNGs for the docs.
- Snapshots: `crates/anvil_ui/tests/component_snapshots.rs`, 10 sheets in 3 themes (30 PNGs in
  `tests/snapshots/components/`): buttons, form controls, feedback, text controls, navigation
  and lists, a virtual list, and the menu, modal, sheet and tooltip-with-toast overlays.
  `ANVIL_UPDATE_SNAPSHOTS=1 cargo test -p anvil_ui --test component_snapshots` regenerates
  them. Behavior is covered by `tests/components.rs` (17 tests, no GPU).
- Not done: the two-column "dark/light side by side" docs page on the website, and drawing
  an input method's composition inside the multi-line editor.

### Phase 4: Migrate ANVL Note (about 2 weeks), section 5

### Phase 5: Retire the old API (about 1 week)

- Kiosk demo (`anvil_app`): migrate its screens, or freeze it on a compatibility module.
- Delete `Column`, `Row`, `ShellLayout` slicing; immediate-mode widgets become internal to the
  components that still use them, or go.
- Docs: ADL-V3 updated to the element API; this file becomes the API guide.

**Total: about 10 to 11 weeks of focused work.** Phases 0 to 2 are the risky ones; 3 to 5 are
volume.

---

## 5. How ANVL Note migrates

Smallest and most self-contained first, the editor last. Each step ships on its own.

| Step | What moves | From (today) | Notes |
|---|---|---|---|
| 1 | Pair a device sheet | `build_pair_sheet` (about 180 lines of rect math) | Self-contained modal: proves forms, text input, toggle, buttons, glass, click-outside |
| 2 | Toasts and overlays | `build_overlays`, file-drop hint | Proves layer 2, stacking, actions (Undo) |
| 3 | Editor toolbar | `build_editor_toolbar` | Proves icon buttons, disabled states, keeping editor focus on press |
| 4 | Phone list | `build_phone_list`, `search_field`, `row_place`, `empty_state`, `swipe_reveal` | `uniform_list`, swipe gesture, entrance stagger, nav bar. `note_row` becomes Note's own component |
| 5 | Desktop sidebar | `build_sidebar` | Same components as step 4 in the sidebar layout |
| 6 | Header and status bar | `anvil_header`, `status_bar` (in `anvil_ui/src/ui/anvl.rs`) | Move into `apps/anvil_note`; the engine loses its app-specific code |
| 7 | Frame shell | `frame()` layout (`ShellLayout`, insets, screen slide) | Root becomes `div().flex_row()` (Expanded/Medium) or a screen stack (Compact) |
| 8 | Editor | `text_editor` | Wrapped as a `text_editor` component: same editing core, laid out by Taffy |

After step 8, Note's `app.rs` keeps its state, sync and lifecycle; its view becomes
`fn render(&self, cx) -> Element<Msg>` plus `fn update(&mut self, msg)`.

Parity checks on every step: the step's screen gets headless snapshots before it moves, and must
match afterwards (or differ only where the change is intended and noted); touch behavior (slop,
long press, swipe thresholds) keeps its existing tests.

---

## 6. Risks

| Risk | Mitigation |
|---|---|
| Per-frame rebuild cost on phones | Reuse Taffy and arena allocations; cache text by `TextKey`; `uniform_list`; profile on the phone at the end of Phase 1 |
| Action model feels limiting | Local element state with closures covers view-only state; revisit entities only if a real screen needs them |
| Two APIs at once for weeks | The bridges (`ui.element`, `canvas`) are tested first, in Phase 1 |
| Behavior drift (touch, scroll, focus) | Existing interaction tests move with the code; snapshots before each migration step |
| IME and accessibility arrive late | Events are designed for IME composition now (Phase 2); the element tree is what an accessibility tree (AccessKit) will be built from, after Phase 5 |

---

## 7. Decisions needed before Phase 0

1. **Action model** (recommended) or GPUI-style entities.
2. **Light theme** in Phase 0 (more visible early) or Phase 3 (recommended: themes land with the
   components that must look right in them).
3. **Kiosk demo** (`anvil_app`): migrate in Phase 5, or freeze it on a compatibility module.
4. **Taffy 0.14** with flexbox, grid and block layout (recommended), or flexbox only.
