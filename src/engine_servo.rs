//! Servo backend for [`crate::engine`] (cargo feature `servo`).
//!
//! Servo renders into an offscreen software buffer. The host (main.rs) draws that buffer under
//! the toolbar and forwards mouse and keyboard input, so there are no child windows. Everything
//! here runs on the main thread: Servo and its views are not `Send`.
//!
//! Known limits of this first version (see docs/ENGINE_2_0_PLAN.md):
//! - one profile per process: personas share cookies; no helper views (Google account probe),
//!   no docked DevTools, no thumbnails, no `window.open` popups, no audio/video;
//! - `window.ipc.postMessage` is emulated through `console.log("__ipc__" + message)`.

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, mpsc},
    time::Duration,
};

use dpi::PhysicalSize;
use servo::{
    Code, ConsoleLogLevel, CreateNewWebViewRequest, DeviceIntPoint, DeviceIntRect, DevicePoint,
    EventLoopWaker, InputEvent, JSValue, Key, KeyState, KeyboardEvent, LoadStatus, Location, Modifiers,
    MouseButton, MouseButtonAction, MouseButtonEvent, MouseLeftViewportEvent, MouseMoveEvent, NavigationRequest,
    Opts, PermissionFeature, PermissionRequest, Preferences, RenderingContext, Servo, ServoBuilder,
    SoftwareRenderingContext, UserContentManager, UserScript, WebView, WebViewBuilder, WebViewDelegate,
    WheelDelta, WheelEvent, WheelMode,
};
use tao::window::Window;
use url::Url;

use crate::engine::{
    Bounds, EngineError, LoadPhase, MemLevel, PageEngine, PageFrame, PageInput, PageView, Perm, Result, Rgba, Role,
    SnapshotRequest, Source, ViewConfig, ViewHandlers,
};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";

/// Pages post to the browser with `window.ipc.postMessage`; Servo has no such channel, so the
/// shim writes to the console and `ViewDelegate::show_console_message` picks it up.
const IPC_SHIM: &str = "window.ipc = { postMessage: function (m) { try { console.log('__ipc__' + m); } catch (_) {} } };";
const IPC_PREFIX: &str = "__ipc__";

pub type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
struct Waker(Wake);

impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }
    fn wake(&self) {
        (self.0)();
    }
}

/// State shared by the engine and all of its views.
struct Shared {
    scale: Cell<f64>,
    animating: Cell<bool>,
    wake: Wake,
}

pub struct ServoEngine {
    servo: Servo,
    rc: Rc<SoftwareRenderingContext>,
    shared: Rc<Shared>,
}

impl ServoEngine {
    /// `size` is the content area in physical pixels; `scale` the window's scale factor.
    pub fn new(data_dir: PathBuf, size: (u32, u32), scale: f64, wake: Wake) -> Result<Self> {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let size = PhysicalSize::new(size.0.max(1), size.1.max(1));
        let rc = Rc::new(SoftwareRenderingContext::new(size).map_err(|e| EngineError(format!("{e:?}")))?);
        let _ = rc.make_current();
        let opts = Opts { config_dir: Some(data_dir.join("servo")), ..Opts::default() };
        let prefs = Preferences { user_agent: USER_AGENT.to_owned(), ..Preferences::default() };
        let servo = ServoBuilder::default()
            .opts(opts)
            .preferences(prefs)
            .event_loop_waker(Box::new(Waker(wake.clone())))
            .build();
        Ok(ServoEngine { servo, rc, shared: Rc::new(Shared { scale: Cell::new(scale), animating: Cell::new(false), wake }) })
    }
}

impl PageEngine for ServoEngine {
    fn create_view(&mut self, _parent: &Window, config: ViewConfig<'_>, handlers: ViewHandlers) -> Result<Box<dyn PageView>> {
        if config.role == Role::Helper {
            // The Google account probe and the DevTools frontend need extra views and a
            // second profile, which this first Servo version does not have.
            return Err(EngineError("helper views are not supported by the Servo engine".into()));
        }
        let url = match config.content {
            Source::Url(u) => Url::parse(u).map_err(|e| EngineError(e.to_string()))?,
            Source::Html(h) => Url::parse(&format!("data:text/html;charset=utf-8,{}", percent_encode(h)))
                .map_err(|e| EngineError(e.to_string()))?,
        };
        let ucm = Rc::new(UserContentManager::new(&self.servo));
        for script in std::iter::once(IPC_SHIM.to_owned()).chain(config.init_scripts) {
            ucm.add_script(Rc::new(UserScript::new(script, None)));
        }
        let state = Rc::new(ViewState::default());
        let delegate = Rc::new(ViewDelegate { handlers, state: state.clone(), shared: self.shared.clone() });
        let wv = WebViewBuilder::new(&self.servo, self.rc.clone())
            .url(url)
            .hidpi_scale_factor(euclid::Scale::new(self.shared.scale.get() as f32))
            .user_content_manager(ucm)
            .delegate(delegate)
            .build();
        if !config.visible {
            wv.hide();
        }
        let view = ServoView { wv, servo: self.servo.clone(), rc: self.rc.clone(), shared: self.shared.clone(), state, applied_scale: Cell::new(self.shared.scale.get()) };
        view.apply_bounds(config.bounds);
        Ok(Box::new(view))
    }

    fn pump(&mut self) -> Option<Duration> {
        self.servo.spin_event_loop();
        self.shared.animating.get().then_some(Duration::from_millis(16))
    }

    fn set_scale(&mut self, scale: f64) {
        self.shared.scale.set(scale);
    }

    fn spawn_snapshots(&mut self, _on_snapshot: Box<dyn Fn(u32, tiny_skia::Pixmap) + Send>) -> mpsc::Sender<SnapshotRequest> {
        // No thumbnails yet: swallow the requests so senders never fail.
        let (tx, rx) = mpsc::channel::<SnapshotRequest>();
        std::thread::spawn(move || while rx.recv().is_ok() {});
        tx
    }
}

#[derive(Default)]
struct ViewState {
    dirty: Cell<bool>,
    url: RefCell<String>,
    last: RefCell<Option<Rc<PageFrame>>>,
}

struct ViewDelegate {
    handlers: ViewHandlers,
    state: Rc<ViewState>,
    shared: Rc<Shared>,
}

impl WebViewDelegate for ViewDelegate {
    fn notify_new_frame_ready(&self, _wv: WebView) {
        self.state.dirty.set(true);
        (self.shared.wake)();
    }

    fn notify_animating_changed(&self, _wv: WebView, animating: bool) {
        self.shared.animating.set(animating);
    }

    fn notify_url_changed(&self, _wv: WebView, url: Url) {
        *self.state.url.borrow_mut() = url.to_string();
    }

    fn notify_page_title_changed(&self, _wv: WebView, title: Option<String>) {
        if let Some(f) = &self.handlers.on_title {
            f(title.unwrap_or_default());
        }
    }

    fn notify_load_status_changed(&self, _wv: WebView, status: LoadStatus) {
        let phase = match status {
            LoadStatus::Started => LoadPhase::Started,
            LoadStatus::Complete => LoadPhase::Finished,
            _ => return,
        };
        if let Some(f) = &self.handlers.on_load {
            f(phase, self.state.url.borrow().clone());
        }
    }

    fn request_navigation(&self, _wv: WebView, request: NavigationRequest) {
        let allowed = self.handlers.on_navigate.as_ref().is_none_or(|f| f(request.url.as_str()));
        if allowed {
            request.allow();
        } else {
            request.deny();
        }
    }

    fn request_permission(&self, _wv: WebView, request: PermissionRequest) {
        let perm = match request.feature() {
            PermissionFeature::Camera => Perm::Camera,
            PermissionFeature::Microphone => Perm::Microphone,
            PermissionFeature::Geolocation => Perm::Geolocation,
            PermissionFeature::Notifications => Perm::Notifications,
            _ => Perm::Other,
        };
        if self.handlers.on_permission.as_ref().is_some_and(|f| f(perm)) {
            request.allow();
        } else {
            request.deny();
        }
    }

    // Dropping the request refuses the new window (the page's `window.open`).
    fn request_create_new(&self, _wv: WebView, _request: CreateNewWebViewRequest) {}

    fn show_console_message(&self, _wv: WebView, _level: ConsoleLogLevel, message: String) {
        if let Some(body) = message.strip_prefix(IPC_PREFIX)
            && let Some(f) = &self.handlers.on_message
        {
            f(body);
        }
    }
}

struct ServoView {
    wv: WebView,
    servo: Servo,
    rc: Rc<SoftwareRenderingContext>,
    shared: Rc<Shared>,
    state: Rc<ViewState>,
    applied_scale: Cell<f64>,
}

impl ServoView {
    /// Only a real content area resizes the shared buffer; parked 1x1 views must not shrink it.
    fn apply_bounds(&self, b: Bounds) {
        if b.w <= 1.0 || b.h <= 1.0 {
            return;
        }
        let scale = self.shared.scale.get();
        if scale != self.applied_scale.get() {
            self.applied_scale.set(scale);
            self.wv.set_hidpi_scale_factor(euclid::Scale::new(scale as f32));
        }
        let (w, h) = (((b.w * scale).round() as u32).max(1), ((b.h * scale).round() as u32).max(1));
        if self.rc.size() != PhysicalSize::new(w, h) {
            self.wv.resize(PhysicalSize::new(w, h));
            self.state.dirty.set(true);
        }
    }
}

impl PageView for ServoView {
    fn load_url(&self, url: &str) -> Result<()> {
        self.wv.load(Url::parse(url).map_err(|e| EngineError(e.to_string()))?);
        Ok(())
    }
    fn reload(&self) -> Result<()> {
        self.wv.reload();
        Ok(())
    }
    fn go_back(&self) -> Result<()> {
        self.wv.go_back(1);
        Ok(())
    }
    fn go_forward(&self) -> Result<()> {
        self.wv.go_forward(1);
        Ok(())
    }
    fn can_go_back(&self) -> Result<bool> {
        Ok(self.wv.can_go_back())
    }
    fn can_go_forward(&self) -> Result<bool> {
        Ok(self.wv.can_go_forward())
    }
    fn set_bounds(&self, bounds: Bounds) -> Result<()> {
        self.apply_bounds(bounds);
        Ok(())
    }
    fn set_visible(&self, visible: bool) -> Result<()> {
        if visible {
            self.wv.show();
            self.state.dirty.set(true);
        } else {
            self.wv.hide();
        }
        Ok(())
    }
    fn set_memory_level(&self, _level: MemLevel) -> Result<()> {
        Ok(())
    }
    fn set_background(&self, _color: Rgba) -> Result<()> {
        Ok(())
    }
    fn focus(&self) -> Result<()> {
        self.wv.set_focused(true);
        Ok(())
    }
    fn focus_parent(&self) -> Result<()> {
        self.wv.set_focused(false);
        Ok(())
    }
    fn eval(&self, js: &str) -> Result<()> {
        self.wv.evaluate_javascript(js, |_| {});
        Ok(())
    }
    fn eval_with_result(&self, js: &str, callback: Box<dyn Fn(String) + Send>) -> Result<()> {
        self.wv.evaluate_javascript(js, move |result| {
            if let Ok(value) = result {
                callback(js_to_json(&value));
            }
        });
        Ok(())
    }
    fn clear_browsing_data(&self) -> Result<()> {
        self.servo.site_data_manager().clear_cookies(None);
        Ok(())
    }
    fn devtools_frontend(&self, _seq: u64, _page_url: &str, done: Box<dyn FnOnce(Option<String>) + Send>) -> Result<()> {
        done(None);
        Ok(())
    }
    fn devtools_lookup_finished(&self) {}
    fn open_devtools(&self) {}

    fn host_painted(&self) -> bool {
        true
    }

    fn paint(&self) -> Option<Rc<PageFrame>> {
        if self.state.dirty.replace(false) || self.state.last.borrow().is_none() {
            self.wv.paint();
            let size = self.rc.size();
            let rect = DeviceIntRect::new(
                DeviceIntPoint::new(0, 0),
                DeviceIntPoint::new(size.width as i32, size.height as i32),
            );
            if let Some(img) = self.rc.read_to_image(rect) {
                let (width, height) = (img.width(), img.height());
                *self.state.last.borrow_mut() = Some(Rc::new(PageFrame { width, height, rgba: img.into_raw() }));
            }
        }
        self.state.last.borrow().clone()
    }

    fn input(&self, event: PageInput) {
        let event = match event {
            PageInput::MouseMove { x, y } => InputEvent::MouseMove(MouseMoveEvent::new(DevicePoint::new(x, y).into())),
            PageInput::MouseLeave => InputEvent::MouseLeftViewport(MouseLeftViewportEvent::default()),
            PageInput::MouseButton { down, button, x, y } => {
                let action = if down { MouseButtonAction::Down } else { MouseButtonAction::Up };
                let button = match button {
                    0 => MouseButton::Primary,
                    1 => MouseButton::Auxiliary,
                    2 => MouseButton::Secondary,
                    3 => MouseButton::Back,
                    4 => MouseButton::Forward,
                    n => MouseButton::Other(u16::from(n)),
                };
                InputEvent::MouseButton(MouseButtonEvent::new(action, button, DevicePoint::new(x, y).into()))
            }
            PageInput::Wheel { dx, dy, line, x, y } => {
                let (k, mode) = if line { (76.0, WheelMode::DeltaLine) } else { (1.0, WheelMode::DeltaPixel) };
                InputEvent::Wheel(WheelEvent::new(
                    WheelDelta { x: dx * k, y: dy * k, z: 0.0, mode },
                    DevicePoint::new(x, y).into(),
                ))
            }
            PageInput::Key { down, key, code, mods, repeat } => {
                let key = key.parse::<Key>().unwrap_or(Key::Character(key));
                let code = code.parse::<Code>().unwrap_or(Code::Unidentified);
                let mut m = Modifiers::empty();
                if mods & 1 != 0 {
                    m |= Modifiers::SHIFT;
                }
                if mods & 2 != 0 {
                    m |= Modifiers::CONTROL;
                }
                if mods & 4 != 0 {
                    m |= Modifiers::ALT;
                }
                if mods & 8 != 0 {
                    m |= Modifiers::META;
                }
                let state = if down { KeyState::Down } else { KeyState::Up };
                InputEvent::Keyboard(KeyboardEvent::new_without_event(state, key, code, Location::Standard, m, repeat, false))
            }
        };
        self.wv.notify_input_event(event);
    }
}

/// The JSON text of a script result, the shape `eval_with_result` callers expect.
fn js_to_json(v: &JSValue) -> String {
    match v {
        JSValue::Undefined | JSValue::Null => "null".into(),
        JSValue::Boolean(b) => b.to_string(),
        JSValue::Number(n) => serde_json::Number::from_f64(*n).map_or("null".into(), |n| n.to_string()),
        JSValue::String(s) => serde_json::to_string(s).unwrap_or_else(|_| "null".into()),
        JSValue::Array(items) => format!("[{}]", items.iter().map(js_to_json).collect::<Vec<_>>().join(",")),
        JSValue::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(k, v)| format!("{}:{}", serde_json::to_string(k).unwrap_or_default(), js_to_json(v)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        _ => "null".into(),
    }
}

/// Percent-encodes everything outside the URL-safe set, for `data:` URLs.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::percent_encode;

    #[test]
    fn encodes_data_url_body() {
        assert_eq!(percent_encode("<a b>"), "%3Ca%20b%3E");
    }
}
