# ANVIL Agent Guide

Instructions for an AI agent (or a person) building a desktop app on the ANVIL engine. Read all of it before writing code. If something is not listed under "Built", it does not exist: do not invent it.

Workspace crates: `anvl_gpu` (renderer, text, icons, images), `anvil_ui` (elements, components, layout, theme; re-exports `anvl_gpu`), `anvil_core` (kiosk data model, no GPU). **Depend on `anvil_ui` only** and use `anvil_ui::*` paths, not `anvl_gpu::*`. Public API rules: `API-POLICY.md`.

## 1. Start

```bash
cargo run -p anvl_starter
```

`apps/anvl_starter` is the template: copy it, rename the crate, keep the shape of `app.rs`. Verify with `cargo test -p anvl_starter` and `cargo test -p anvil_ui`.

## 2. The model (every app follows it)

1. State lives in your `App` struct. Controls never own state.
2. Each frame: `ui.begin(input, screen, scale, time, dt)`, build an element tree from the state, `ui.element(area, tree)`, `ui.end()`.
3. Controls carry a message value (`.on_click(Msg::X)`). After the frame, `ui.take_actions::<Msg>()` returns the messages clicked this frame. Apply them to the state in `update`. The next frame shows the change.
4. The tree is rebuilt every frame and is cheap. Give list rows stable ids: `ElementId::new("row").key(id)`.
5. Text inputs edit a `&mut String` in place, so the view borrows the text it edits. Pass the field, not the whole state, to the view function.
6. Window, GPU and frame plumbing is in `apps/anvl_starter/src/app.rs` (`resumed`, `window_event`, `frame`, `about_to_wait`). Copy it; do not rewrite it. The runner (`anvil_ui::runner`) puts the app on its own UI thread.

Use `use anvil_ui::*;` and `use anvil_ui::components::*;`. Layout units are logical px. Spacing steps are 4 px: `gap_3` is 12, `p_4` is 16.

## 3. Built: elements

Constructors: `div()`, `text(s)`, `icon(Icon::X)`, `img(TextureId)`, `canvas(|ui, rect| ...)` (immediate-mode drawing inside the tree), `uniform_list(id, count, row_height, |i| row)` (virtualized, fixed row height).

`Element` methods: `child children id flex_row flex_col flex_wrap grid_cols col_span hidden items_start items_center items_end items_stretch items_baseline self_center justify_start justify_center justify_end justify_between justify_around gap gap_x gap_y p px py pt pb m mt my mb ml mr w h size w_full h_full w_frac min_w max_w min_h max_h flex_1 grow shrink_0 absolute top right bottom left bg border_1 border_color rounded rounded_sm rounded_md rounded_lg rounded_full shadow glass opacity overflow_hidden color text_size medium semibold italic mono nowrap body heading title caption label fit`, plus `.scroll()`, `.hover(|s| ..)`, `.active(|s| ..)`, `.transition(motion::INSTANT)`, `.enter_at(Enter::Rise, i)`, `.on_click(msg)`, and the spacing steps `gap_1..`, `p_1..`, `px_3`, and so on. Colors are tokens: `Tok::Bg Card CardHi Border Text Muted Primary PrimaryFg Danger` (more in `theme`).

## 4. Built: components (`anvil_ui::components`)

Every component also takes `.id(..)`, `.disabled(bool)`, `.tooltip(..)`, `.pin(..)`. Click handlers take your message value; change handlers take a closure that returns one.

| Component | Constructor | Props |
|---|---|---|
| Button | `button(label)` | `.primary() .secondary() .ghost() .destructive() .success() .variant(Variant) .icon(Icon) .full_width() .on_click(msg)` |
| IconButton | `icon_button(Icon)` | `.selected(bool) .on_click(msg)` |
| TextInput | `text_input(id, &mut String)` | `.placeholder(s) .icon(Icon) .kind(InputKind) .max_len(n) .disabled(bool)` |
| SearchField | `search_field(id, &mut String)` | same as TextInput |
| TextEditor (multi-line) | `text_editor(id, &mut String, &mut EditorCursor)` | `.placeholder(s) .output(&mut EditorOutput)` |
| Checkbox | `checkbox(checked)` | `.label(s) .on_click(msg)` |
| Toggle | `toggle(on)` | `.on_click(msg)` |
| Slider | `slider(value 0..1)` | `.tone(Tone) .on_change(|v| msg)` |
| Segmented | `segmented(labels, selected)` | `.segment_width(px) .on_select(|i| msg)` |
| Tabs | `tabs(labels, selected)` | `.on_select(|i| msg)` |
| NavBar | `nav_bar(&[NavItem], selected)` | `.on_select(|i| msg)` |
| Toolbar | `toolbar(items)`, `toolbar_divider()` | items are elements |
| ListItem | `list_item(title)` | `.subtitle(s) .icon(Icon) .trailing(el) .selected(bool) .on_click(msg)` |
| Menu | `menu([MenuItem])` | `.width(px) .on_dismiss(msg) .at(x, y)`; `MenuItem::new(label)` with `.icon .shortcut .danger() .disabled .on_click`, and `MenuItem::separator()` |
| Modal | `modal(open, content)` | `.width(px) .on_dismiss(msg)` |
| Sheet | `sheet(open, content)` | `.on_dismiss(msg)` |
| Tooltip | `tooltip(element, text)` | |
| ToastStack | `toast_stack(&mut Toasts)` | `.on_action(|id| msg)`; add toasts with `Toasts::push_action(title, body, color, action_label)` |
| Badge | `badge(label)` | `.icon(Icon)`; tones via `Tone` |
| Progress | `progress(0..1)` | `.ring(size)` |
| Skeleton | `skeleton()` | |
| EmptyState | `empty_state(Icon, title)` | `.body(s) .action(element)` |

Also built: virtualized `uniform_list`, scrolling `div().scroll()`, three themes (`Theme::adl_dark() adl_light() high_contrast()`, switch with `ui.switch_theme`), Markdown highlighting in the editor, images and video (`decode_image`, `Renderer::upload`, `Renderer::release`), clipboard, soft keyboard, toasts with Undo, and the immediate-mode widgets for use inside `canvas` (`ui.text_field`, `ui.button`, `ui.slider`, `ui.toggle`, `ui.select` (a dropdown), `ui.badge`, `ui.progress`).

Textures: `Renderer::upload(&Frame) -> TextureId`. Call `Renderer::release(id)` when you are done with a texture; the slot is reused. Never draw or update a released id.

## 5. NOT built (do not use, do not invent)

- Tree view, accordion, multi-select, date picker, time picker. Compose them from `div`, `list_item` and `Menu` yourself.
- A dropdown **element** component. Only the immediate-mode `ui.select` exists, inside a `canvas`.
- Variable-height virtualized lists. `uniform_list` rows share one height.
- Right-to-left layout and bidirectional text. Locale-aware formatting. String tables.
- IME composition shown in the multi-line editor (it shows committed text only).
- Accessibility tree and screen reader support.
- Theme files loaded from disk, and runtime user palettes. Themes are the three built-in constants.
- Backdrop blur on the lowest layer, Mica or Acrylic window materials, haptics, count-up animation.
- Rich-text or image paste. Drag and drop of files.
- Hot reload, an `anvil` CLI, a component registry.
- iOS and web (WASM). Desktop and Android only; Windows is the tested desktop target.

If you need something in this list, say so, and build it in the app crate from `div` and `canvas`.

## 6. Rules that prevent most breakage

- A text field consumes Enter. To submit on Enter, read `ui.is_focused(ID) && ui.key_pressed(UiKey::Enter)` **before** building the field.
- Every list row, and every control inside a repeated block, needs a unique, stable id.
- Never call GPU code from `update`. Never hold a `&mut String` borrow across the call to `update`.
- No `unwrap()` or `expect()` on runtime paths. `Renderer::new` returns `Result<Renderer, GpuError>`; on `Err`, log it and keep running without drawing.
- Idle apps must sleep: only request another frame while `ui.next_wake()` is `Some`.
- Add a test that drives the UI headless (see the tests named below and `apps/anvl_starter/src/app.rs`). Use `Input { mouse, pressed, down, released, typed, keys, .. }` and `ui.take_actions`.

## 7. Example 1: a form with inputs and a submit button

Compiled and tested in `crates/anvil_ui/tests/agent_examples.rs`. The `Rig` there is a ten-line harness: `ui.begin`, `ui.element`, `ui.end`, `ui.take_actions`.

```rust
use anvil_ui::components::*;
use anvil_ui::*;

#[derive(Clone, Debug, PartialEq)]
enum FormMsg {
    SetNewsletter(bool),
    Submit,
}

#[derive(Default)]
struct FormState {
    name: String,
    email: String,
    newsletter: bool,
    submitted: Option<String>,
}

impl FormState {
    fn update(&mut self, msg: FormMsg) {
        match msg {
            FormMsg::SetNewsletter(on) => self.newsletter = on,
            FormMsg::Submit => {
                self.submitted = Some(format!("{} <{}> newsletter={}", self.name, self.email, self.newsletter));
            }
        }
    }
}

/// The view borrows the text it edits, so it takes `&mut` of the two strings, not the whole state.
fn form_view<'a>(name: &'a mut String, email: &'a mut String, newsletter: bool) -> Element<'a> {
    div()
        .p_4()
        .gap_3()
        .bg(Tok::Bg)
        .child(div().h(44.0).child(text_input("name", name).placeholder("Name")))
        .child(div().h(44.0).child(text_input("email", email).placeholder("Email")))
        .child(div().h(44.0).child(checkbox(newsletter).label("Send me news").on_click(FormMsg::SetNewsletter(!newsletter))))
        .child(div().h(44.0).child(button("Submit").primary().full_width().on_click(FormMsg::Submit)))
}
```

## 8. Example 2: a stateful counter and list

```rust
use anvil_ui::components::*;
use anvil_ui::element::uniform_list;
use anvil_ui::*;

#[derive(Clone, Debug, PartialEq)]
enum CounterMsg {
    Increment,
    Remove(usize),
}

#[derive(Default)]
struct CounterState {
    items: Vec<u32>,
    next: u32,
}

impl CounterState {
    fn update(&mut self, msg: CounterMsg) {
        match msg {
            CounterMsg::Increment => {
                self.next += 1;
                self.items.push(self.next);
            }
            CounterMsg::Remove(i) => {
                if i < self.items.len() {
                    self.items.remove(i);
                }
            }
        }
    }
}

fn counter_view(state: &CounterState) -> Element<'_> {
    div()
        .p_4()
        .gap_3()
        .bg(Tok::Bg)
        .child(text(format!("{} items", state.items.len())).heading())
        .child(div().h(44.0).child(button("Add item").primary().on_click(CounterMsg::Increment)))
        .child(
            uniform_list("items", state.items.len(), 48.0, |i| {
                list_item(format!("Item {}", state.items[i])).id(ElementId::new("item").key(i as u64)).on_click(CounterMsg::Remove(i))
            })
            .flex_1(),
        )
}
```

In a real app, call these from `App::build` exactly as `apps/anvl_starter/src/app.rs` does: `ui.element(area, view(..))`, then `for msg in ui.take_actions::<Msg>() { self.update(msg) }`.

## 9. Checklist before you say "done"

1. `cargo build -p <your_app>` has no warnings you introduced.
2. `cargo test -p <your_app>` and `cargo test -p anvil_ui` pass.
3. Every widget you used is in section 3 or 4.
