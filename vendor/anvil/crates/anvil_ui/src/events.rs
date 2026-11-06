//! Turns winit's window events into the engine's per-frame [`Input`]. Every app needs this, so
//! it lives here: feed it each `WindowEvent` as it arrives, and take one `Input` per frame.
//!
//! ```ignore
//! fn window_event(&mut self, event: WindowEvent, cx: &mut Cx) {
//!     if self.input.event(&event, self.scale) { cx.request_redraw(); }
//! }
//! fn frame(&mut self, _: &mut Cx) {
//!     if self.input.wants_paste() { self.input.set_paste(self.clipboard.get_text()); }
//!     self.ui.begin(self.input.take(), size, scale, time, dt);
//!     // ...
//! }
//! ```

use glam::Vec2;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::keyboard::{Key, NamedKey};
use winit::window::CursorIcon;

use crate::ui::{Cursor, Input, Preedit, UiKey};

/// Pixels one wheel notch scrolls, in logical px.
const LINE: f32 = 48.0;

/// An input method's state for one window: what it is composing, and whether it is switched on.
/// `WindowInput` has one; an app that translates window events itself keeps its own, feeds it
/// each `WindowEvent::Ime` with [`ImeState::event`], puts [`ImeState::preedit`] in each frame's
/// `Input`, and calls [`ImeState::sync`] after building the frame.
#[derive(Default)]
pub struct ImeState {
    preedit: Option<Preedit>,
    allowed: bool,
}

impl ImeState {
    /// Handles one input-method event. Returns the text it committed, if any: type it into the
    /// UI as `Input::typed`.
    pub fn event(&mut self, ime: &Ime) -> Option<String> {
        match ime {
            Ime::Preedit(text, cursor) => {
                self.preedit = (!text.is_empty()).then(|| Preedit { text: text.clone(), cursor: *cursor });
                None
            }
            Ime::Commit(text) => {
                self.preedit = None;
                Some(text.chars().filter(|c| !c.is_control()).collect())
            }
            Ime::Enabled => None,
            Ime::Disabled => {
                self.preedit = None;
                None
            }
        }
    }

    /// What is being composed now, for `Input::preedit`.
    pub fn preedit(&self) -> Option<Preedit> {
        self.preedit.clone()
    }

    /// Keeps the window's input method in step with the UI: on while a text field has focus,
    /// with its candidate window beside the caret (`Ui::ime_area`). Call it once per frame
    /// after the UI is built.
    pub fn sync(&mut self, window: &winit::window::Window, area: Option<crate::geom::Rect>) {
        let want = area.is_some();
        if want != self.allowed {
            self.allowed = want;
            window.set_ime_allowed(want);
            if !want {
                self.preedit = None;
            }
        }
        if let Some(r) = area {
            window.set_ime_cursor_area(
                winit::dpi::PhysicalPosition::new(r.x() as f64, r.y() as f64),
                winit::dpi::PhysicalSize::new(r.w().max(1.0) as f64, r.h().max(1.0) as f64),
            );
        }
    }
}

/// Collects window events between frames.
#[derive(Default)]
pub struct WindowInput {
    input: Input,
    pointer: Vec2,
    down: bool,
    shift: bool,
    ctrl: bool,
    alt: bool,
    touch: bool,
    file_hover: bool,
    paste: bool,
    back: bool,
    shortcuts: Vec<char>,
    ime: ImeState,
}

impl WindowInput {
    pub fn new() -> Self {
        Self { pointer: Vec2::splat(-1.0), ..Default::default() }
    }

    /// Handles one window event. `scale` is the window's scale factor (for wheel lines).
    /// Returns true when the UI should draw again.
    pub fn event(&mut self, event: &WindowEvent, scale: f32) -> bool {
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.touch = false;
                self.pointer = Vec2::new(position.x as f32, position.y as f32);
            }
            WindowEvent::CursorLeft { .. } => self.pointer = Vec2::splat(-1.0),
            WindowEvent::Touch(t) => {
                self.touch = true;
                self.pointer = Vec2::new(t.location.x as f32, t.location.y as f32);
                match t.phase {
                    TouchPhase::Started => self.press(),
                    TouchPhase::Ended | TouchPhase::Cancelled => self.release(),
                    TouchPhase::Moved => {}
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                if *state == ElementState::Pressed {
                    self.press();
                } else {
                    self.release();
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } => {
                self.input.secondary = true;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.input.scroll += match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * LINE * scale,
                    MouseScrollDelta::PixelDelta(p) => -(p.y as f32),
                };
            }
            WindowEvent::ModifiersChanged(m) => {
                let s = m.state();
                self.shift = s.shift_key();
                self.ctrl = s.control_key();
                self.alt = s.alt_key();
                return false;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.key(&event.logical_key, event.text.as_deref(), event.state == ElementState::Pressed, event.repeat);
            }
            WindowEvent::Ime(ime) => {
                if let Some(text) = self.ime.event(ime) {
                    self.input.typed.push_str(&text);
                }
            }
            WindowEvent::HoveredFile(_) => self.file_hover = true,
            WindowEvent::HoveredFileCancelled => self.file_hover = false,
            WindowEvent::DroppedFile(path) => {
                self.file_hover = false;
                self.input.dropped.push(path.display().to_string());
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } | WindowEvent::Focused(_) => {}
            _ => return false,
        }
        true
    }

    fn press(&mut self) {
        self.input.pressed = true;
        self.down = true;
    }

    fn release(&mut self) {
        self.input.released = true;
        self.down = false;
    }

    fn key(&mut self, key: &Key, text: Option<&str>, pressed: bool, repeat: bool) {
        // Android's key character map turns Enter and Tab into the characters "\n" and "\t", so
        // winit reports them as text. Treat them as the keys.
        let named;
        let (key, text) = match key {
            Key::Character(c) if matches!(c.as_str(), "\n" | "\r" | "\t") => {
                named = Key::Named(if c == "\t" { NamedKey::Tab } else { NamedKey::Enter });
                (&named, None)
            }
            _ => (key, text),
        };

        // Ctrl with a letter is a shortcut. Ctrl+Alt is AltGr on many layouts and types text.
        if pressed && self.ctrl && !self.alt {
            if let Key::Character(c) = key {
                if !repeat {
                    let lower = c.to_lowercase();
                    match lower.as_str() {
                        "a" => self.input.keys.push(UiKey::SelectAll),
                        "c" => self.input.keys.push(UiKey::Copy),
                        "x" => self.input.keys.push(UiKey::Cut),
                        "s" => self.input.keys.push(UiKey::Save),
                        "k" => self.input.keys.push(UiKey::Command),
                        "z" => self.input.keys.push(if self.shift { UiKey::Redo } else { UiKey::Undo }),
                        "y" => self.input.keys.push(UiKey::Redo),
                        "v" => self.paste = true,
                        other => self.shortcuts.extend(other.chars().next()),
                    }
                }
                return;
            }
        }

        if pressed && !repeat && matches!(key, Key::Named(NamedKey::GoBack | NamedKey::BrowserBack)) {
            self.back = true;
            return;
        }

        if let Key::Named(named) = key {
            let ui_key = match named {
                NamedKey::Backspace => Some(UiKey::Backspace),
                NamedKey::Delete => Some(UiKey::Delete),
                NamedKey::Enter => Some(UiKey::Enter),
                NamedKey::Escape => Some(UiKey::Escape),
                NamedKey::Tab => Some(if self.shift { UiKey::BackTab } else { UiKey::Tab }),
                NamedKey::Space => Some(UiKey::Space),
                NamedKey::Home => Some(UiKey::Home),
                NamedKey::End => Some(UiKey::End),
                NamedKey::ArrowUp => Some(UiKey::ArrowUp),
                NamedKey::ArrowDown => Some(UiKey::ArrowDown),
                NamedKey::ArrowLeft => Some(UiKey::ArrowLeft),
                NamedKey::ArrowRight => Some(UiKey::ArrowRight),
                _ => None,
            };
            if let Some(k) = ui_key {
                let repeatable = !matches!(k, UiKey::Escape | UiKey::Space);
                if pressed && (!repeat || repeatable) {
                    self.input.keys.push(k);
                }
                if !pressed && matches!(k, UiKey::Enter | UiKey::Space) {
                    self.input.key_releases.push(k);
                }
            }
        }

        let typed = match (text, key) {
            (Some(text), _) => Some(text),
            (None, Key::Character(c)) => Some(c.as_str()),
            _ => None,
        };
        if let (true, Some(typed)) = (pressed, typed) {
            self.input.typed.extend(typed.chars().filter(|c| !c.is_control()));
        }
    }

    /// Keeps the window's input method in step with the UI: on while a text field has focus,
    /// with its candidate window beside the caret (`Ui::ime_area`). Call it once per frame
    /// after the UI is built.
    pub fn sync_ime(&mut self, window: &winit::window::Window, area: Option<crate::geom::Rect>) {
        self.ime.sync(window, area);
    }

    /// True after Ctrl+V: read the clipboard and pass its text to [`WindowInput::set_paste`]
    /// before [`WindowInput::take`].
    pub fn wants_paste(&self) -> bool {
        self.paste
    }

    pub fn set_paste(&mut self, text: Option<String>) {
        self.paste = false;
        self.input.paste = text;
    }

    /// Ctrl+letter shortcuts the UI does not handle itself (Ctrl+N, Ctrl+F, ...), lowercase.
    pub fn take_shortcuts(&mut self) -> Vec<char> {
        std::mem::take(&mut self.shortcuts)
    }

    /// True once after Android's back button (or a browser-back key).
    pub fn take_back(&mut self) -> bool {
        std::mem::take(&mut self.back)
    }

    pub fn pointer(&self) -> Vec2 {
        self.pointer
    }

    /// The input for the next frame. Clears this frame's presses, keys and text.
    pub fn take(&mut self) -> Input {
        let mut input = std::mem::take(&mut self.input);
        input.mouse = self.pointer;
        input.down = self.down;
        input.shift = self.shift;
        input.ctrl = self.ctrl;
        input.touch = self.touch;
        input.file_hover = self.file_hover;
        input.preedit = self.ime.preedit();
        input
    }
}

/// The OS pointer for what the UI asked for this frame (`Ui::cursor`).
pub fn cursor_icon(cursor: Cursor) -> CursorIcon {
    match cursor {
        Cursor::Arrow => CursorIcon::Default,
        Cursor::Pointer => CursorIcon::Pointer,
        Cursor::Text => CursorIcon::Text,
        Cursor::Grab => CursorIcon::Grab,
        Cursor::Grabbing => CursorIcon::Grabbing,
        Cursor::NotAllowed => CursorIcon::NotAllowed,
        Cursor::ResizeHorizontal => CursorIcon::EwResize,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::dpi::PhysicalPosition;
    use winit::event::DeviceId;

    fn moved(x: f64, y: f64) -> WindowEvent {
        // SAFETY: a dummy device id is only compared, never dereferenced.
        WindowEvent::CursorMoved { device_id: unsafe { DeviceId::dummy() }, position: PhysicalPosition::new(x, y) }
    }

    fn button(pressed: bool) -> WindowEvent {
        WindowEvent::MouseInput {
            device_id: unsafe { DeviceId::dummy() },
            state: if pressed { ElementState::Pressed } else { ElementState::Released },
            button: MouseButton::Left,
        }
    }

    #[test]
    fn a_click_becomes_one_frames_press_and_release_at_the_pointer() {
        let mut w = WindowInput::new();
        assert!(w.event(&moved(10.0, 20.0), 1.0));
        w.event(&button(true), 1.0);
        let i = w.take();
        assert!(i.pressed && i.down && !i.released);
        assert_eq!(i.mouse, Vec2::new(10.0, 20.0));
        w.event(&button(false), 1.0);
        let i = w.take();
        assert!(!i.pressed && !i.down && i.released, "the press is not repeated");
    }

    #[test]
    fn android_enter_as_text_is_the_enter_key_and_ctrl_letters_are_shortcuts() {
        let mut w = WindowInput::new();
        w.key(&Key::Character("\n".into()), Some("\n"), true, false);
        w.key(&Key::Character("h".into()), Some("h"), true, false);
        let i = w.take();
        assert_eq!(i.keys, vec![UiKey::Enter]);
        assert_eq!(i.typed, "h");
        w.ctrl = true;
        w.key(&Key::Character("z".into()), None, true, false);
        w.key(&Key::Character("n".into()), None, true, false);
        w.key(&Key::Character("v".into()), None, true, false);
        assert!(w.wants_paste());
        w.set_paste(Some("clip".into()));
        assert_eq!(w.take_shortcuts(), vec!['n']);
        let i = w.take();
        assert_eq!(i.keys, vec![UiKey::Undo]);
        assert_eq!(i.paste.as_deref(), Some("clip"));
    }
}
