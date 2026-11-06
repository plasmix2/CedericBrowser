//! The seam between the browser and whatever renders web pages.
//!
//! `main.rs` talks only to these traits and types, never to a concrete engine. Today the only
//! backend is WebView2 (`engine_webview2.rs`); a Servo backend is planned
//! (see `docs/ENGINE_2_0_PLAN.md`). Everything here is engine-neutral: no wry, no Chromium flags.
//!
//! Rules a backend must follow (they come from AGENTS.md and apply to every engine):
//! - All views created by one engine share the same options, data folder and look. Backends
//!   apply their own flags inside `create_view`; callers do not pass engine flags.
//! - `create_view` may run a nested message loop, so never call it from inside a page event.
//! - Views start where `ViewConfig::bounds` says and are shown only when `visible` is true.

use std::fmt;

use tao::window::Window;

pub type Rgba = (u8, u8, u8, u8);

/// A rectangle in logical (DPI-independent) pixels, relative to the parent window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Bounds {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Bounds { x, y, w, h }
    }
}

/// Error from an engine. Callers mostly ignore it (`let _ = ...`), as they did with wry's.
#[derive(Debug)]
pub struct EngineError(pub String);

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EngineError {}

pub type Result<T> = std::result::Result<T, EngineError>;

/// How much memory a hidden view may keep. `Low` lets the engine trim caches and suspend work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemLevel {
    Normal,
    Low,
}

/// A page load starting or finishing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadPhase {
    Started,
    Finished,
}

/// What a page is asking permission for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Perm {
    Camera,
    DisplayCapture,
    Microphone,
    Notifications,
    Geolocation,
    Other,
}

/// What a view shows first.
pub enum Source<'a> {
    Url(&'a str),
    Html(&'a str),
}

/// Which kind of view this is. Pages get the full browser feature set; helpers (the Google
/// account probe, the DevTools frontend) are plain surfaces without page-only features.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Page,
    Helper,
}

pub struct ViewConfig<'a> {
    pub role: Role,
    /// The caller's id for this view. Snapshot requests refer to views by it.
    pub tag: u32,
    pub content: Source<'a>,
    pub bounds: Bounds,
    pub visible: bool,
    pub background: Option<Rgba>,
    /// Name of the isolated storage profile (cookies, sign-ins). `None` is the default profile.
    pub profile: Option<String>,
    /// Scripts that run in every document before the page's own scripts.
    pub init_scripts: Vec<String>,
}

/// Callbacks a view reports through. All are optional; the engine denies/ignores what is missing.
#[derive(Default)]
pub struct ViewHandlers {
    /// Return `false` to cancel a navigation to this URL.
    pub on_navigate: Option<Box<dyn Fn(&str) -> bool>>,
    pub on_title: Option<Box<dyn Fn(String)>>,
    pub on_load: Option<Box<dyn Fn(LoadPhase, String)>>,
    /// A page asked to open a new window. The engine always refuses it; the host opens a tab.
    pub on_new_window: Option<Box<dyn Fn(String)>>,
    /// A message the page posted with `window.ipc.postMessage`. The content is untrusted.
    pub on_message: Option<Box<dyn Fn(&str)>>,
    /// Return `true` to grant the permission.
    pub on_permission: Option<Box<dyn Fn(Perm) -> bool + Send + Sync>>,
}

/// Messages for the engine's snapshot worker (tab thumbnails). `u32` is a view's `tag`.
pub enum SnapshotRequest {
    /// The view now on screen (`None` while nothing should be captured, e.g. minimized).
    Active(Option<u32>),
    /// Capture this view now (it just finished loading or was just shown).
    Capture(u32),
    /// The view was closed.
    Forget(u32),
}

/// One rendered frame of a host-painted view: RGBA bytes, `width * height * 4` long.
pub struct PageFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Input for engines that render into a buffer the host draws (`PageView::host_painted`).
/// Coordinates are physical pixels relative to the view's top-left corner.
#[cfg_attr(not(feature = "servo"), allow(dead_code))]
pub enum PageInput {
    MouseMove { x: f32, y: f32 },
    MouseLeave,
    /// `button`: 0 primary, 1 middle, 2 secondary, 3 back, 4 forward.
    MouseButton { down: bool, button: u8, x: f32, y: f32 },
    Wheel { dx: f64, dy: f64, line: bool, x: f32, y: f32 },
    /// `key` and `code` are DOM `KeyboardEvent.key` / `.code` strings. `mods`: 1 shift, 2 ctrl, 4 alt, 8 meta.
    Key { down: bool, key: String, code: String, mods: u8, repeat: bool },
}

/// One page (or helper surface). Dropping it frees the page's renderer and memory.
pub trait PageView {
    fn load_url(&self, url: &str) -> Result<()>;
    fn reload(&self) -> Result<()>;
    fn go_back(&self) -> Result<()>;
    fn go_forward(&self) -> Result<()>;
    fn can_go_back(&self) -> Result<bool>;
    fn can_go_forward(&self) -> Result<bool>;

    fn set_bounds(&self, bounds: Bounds) -> Result<()>;
    fn set_visible(&self, visible: bool) -> Result<()>;
    fn set_memory_level(&self, level: MemLevel) -> Result<()>;
    fn set_background(&self, color: Rgba) -> Result<()>;
    fn focus(&self) -> Result<()>;
    /// Give keyboard focus back to the browser window (for the native address bar).
    fn focus_parent(&self) -> Result<()>;

    /// Run JavaScript in the page and ignore the result.
    fn eval(&self, js: &str) -> Result<()>;
    /// Run JavaScript and pass its JSON-serialized result to `callback`.
    fn eval_with_result(&self, js: &str, callback: Box<dyn Fn(String) + Send>) -> Result<()>;

    /// True if this engine draws into a buffer instead of a child window. The host then calls
    /// `paint` to get the pixels, draws them under the toolbar, and forwards input with `input`.
    fn host_painted(&self) -> bool {
        false
    }
    /// The latest frame (only called for the visible view of a host-painted engine).
    fn paint(&self) -> Option<std::rc::Rc<PageFrame>> {
        None
    }
    fn input(&self, _event: PageInput) {}

    /// Delete cookies, cache and site storage for this view's profile.
    fn clear_browsing_data(&self) -> Result<()>;

    /// Look up the engine's DevTools frontend URL for this page, to dock in a pane. `done` runs once,
    /// possibly on another thread, with `None` if docking is not possible. `seq` is unique per call.
    /// The caller then calls `devtools_lookup_finished` on the UI thread.
    fn devtools_frontend(&self, seq: u64, page_url: &str, done: Box<dyn FnOnce(Option<String>) + Send>) -> Result<()>;
    /// Undo any temporary change `devtools_frontend` made to the page (e.g. its title).
    fn devtools_lookup_finished(&self);

    /// Open the engine's own DevTools window (fallback when docking is unavailable).
    fn open_devtools(&self);
}

/// Creates views and owns state shared by all of them (data folder, profiles).
pub trait PageEngine {
    fn create_view(
        &mut self,
        parent: &Window,
        config: ViewConfig<'_>,
        handlers: ViewHandlers,
    ) -> Result<Box<dyn PageView>>;

    /// Runs the engine's own event loop work. Called when the engine wakes the host (see the
    /// wake callback an engine is built with) and on a timer while it returns a delay.
    fn pump(&mut self) -> Option<std::time::Duration> {
        None
    }
    /// The host window's scale factor changed (host-painted engines need it to size their buffer).
    fn set_scale(&mut self, _scale: f64) {}

    /// Start the worker that takes small snapshots of views for tab hover cards.
    fn spawn_snapshots(&mut self, on_snapshot: Box<dyn Fn(u32, tiny_skia::Pixmap) + Send>) -> std::sync::mpsc::Sender<SnapshotRequest>;
}
