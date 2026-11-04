//! The native browser chrome: tab strip, window controls, toolbar buttons and the address bar,
//! drawn with tiny-skia into the top of the main window. Input arrives as plain window events;
//! what the user asked for comes out as `ChromeCmd`s in `cmds`.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use cosmic_text::{Action, Buffer, Cursor, Edit, Editor, Motion, Selection, Shaping, Weight};
use tao::{
    event::{ElementState, KeyEvent, MouseButton},
    keyboard::{Key, ModifiersState},
    window::CursorIcon,
};
use tiny_skia::Pixmap;

use crate::{
    ChromeCmd, TabId,
    gfx::{Brand, Canvas, Fonts, Icon, Rgba, Theme, attrs, text_width},
};

/// Height of the tab strip (also the window's title bar).
pub const TITLE_H: f32 = 40.0;
/// Total height of the native chrome: tab strip + toolbar.
pub const HEIGHT: f32 = 84.0;

const PAD: f32 = 8.0;
const MAX_W: f32 = 240.0;
const MIN_W: f32 = 48.0;
const NARROW_W: f32 = 88.0;
const TAB_Y: f32 = 6.0;
const TAB_H: f32 = 34.0;
const WIN_BTN_W: f32 = 46.0;
const BTN: f32 = 34.0;
const ROW_Y: f32 = TITLE_H + 5.0;
const FONT: f32 = 12.5;
const URL_FONT: f32 = 13.5;
/// Favicon key for Brave Search pages, drawn as the Cederic "C" badge.
pub const BRAND_ICON: &str = "cederic:brand";

#[derive(Default)]
pub struct TabInfo {
    pub id: TabId,
    pub title: String,
    pub favicon: String,
    pub loading: bool,
    pub pip: bool,
}

#[derive(Default)]
pub struct Model {
    pub tabs: Vec<TabInfo>,
    pub active: TabId,
    pub url: String,
    pub can_back: bool,
    pub can_forward: bool,
    pub loading: bool,
    pub maximized: bool,
    pub devtools: bool,
    pub pip: bool,
    pub bookmarked: bool,
    pub brand: Brand,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Tab(TabId),
    TabClose(TabId),
    NewTab,
    Min,
    Max,
    Close,
    Back,
    Forward,
    Reload,
    Omni,
    Star,
    Bookmarks,
    Pip,
    History,
    Theme,
    Devtools,
    DragArea,
    None,
}

#[derive(Clone, Copy)]
struct R {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl R {
    fn has(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// Animated on-screen state of one tab.
struct TabVis {
    id: TabId,
    x: f32,
    w: f32,
    alpha: f32,
    hover: f32,
    leaving: bool,
    title: Option<(String, Buffer)>,
}

struct Drag {
    id: TabId,
    start_x: f32,
    grab_dx: f32,
    from: usize,
    to: usize,
    active: bool,
}

pub struct Toolbar {
    theme: Theme,
    fonts: Fonts,
    s: f32,
    width: f32,
    model: Model,
    order: Vec<TabId>,
    vis: Vec<TabVis>,
    tab_w: f32,
    frozen_w: Option<f32>,
    keep_order_until: Instant,
    mouse: (f32, f32),
    hover: Hit,
    pressed: Option<Hit>,
    hover_fade: HashMap<u8, f32>,
    drag: Option<Drag>,
    last_click: (Instant, f32, f32, u32),
    progress: f32,
    progress_alpha: f32,
    was_loading: bool,
    load_morph: f32,
    spin: f32,
    icons: HashMap<String, Option<Pixmap>>,
    icon_requests: Vec<String>,
    // address bar
    editing: bool,
    editor: Editor<'static>,
    selecting: bool,
    scroll_x: f32,
    typed: String,
    sugg: Vec<String>,
    sidx: usize,
    blink_from: Instant,
    pretty: Option<(String, Buffer)>,
    placeholder: Buffer,
    clipboard: Option<arboard::Clipboard>,
    // hover cards
    hover_tab: Option<(TabId, Instant, bool)>,
    card_shown: bool,
    card_leave: Option<Instant>,
    last_frame: Instant,
    /// Requests for the browser, drained after each event.
    pub cmds: Vec<ChromeCmd>,
}

fn approach(v: &mut f32, target: f32, k: f32) -> bool {
    let d = target - *v;
    if d.abs() < 0.25 {
        *v = target;
        false
    } else {
        *v += d * k;
        true
    }
}

fn approach01(v: &mut f32, target: f32, k: f32) -> bool {
    let d = target - *v;
    if d.abs() < 0.01 {
        *v = target;
        false
    } else {
        *v += d * k;
        true
    }
}

fn hit_key(h: Hit) -> u8 {
    match h {
        Hit::NewTab => 1,
        Hit::Min => 2,
        Hit::Max => 3,
        Hit::Close => 4,
        Hit::Back => 5,
        Hit::Forward => 6,
        Hit::Reload => 7,
        Hit::Omni => 8,
        Hit::History => 9,
        Hit::Devtools => 10,
        Hit::Star => 11,
        Hit::Bookmarks => 12,
        Hit::Pip => 13,
        Hit::Theme => 14,
        _ => 0,
    }
}

impl Toolbar {
    #[cfg(test)]
    pub fn new(dark: bool, scale: f32, width: f32) -> Self {
        Self::new_with_brand(dark, Brand::Cederic, scale, width)
    }

    pub fn new_with_brand(dark: bool, brand: Brand, scale: f32, width: f32) -> Self {
        let mut fonts = Fonts::load();
        let editor = Editor::new(fonts.line("", URL_FONT, Weight::NORMAL, scale));
        let placeholder = fonts.line("Search or type a URL", URL_FONT, Weight::NORMAL, scale);
        let now = Instant::now();
        Self {
            theme: Theme::for_brand(dark, brand),
            fonts,
            s: scale,
            width,
            model: Model { brand, ..Model::default() },
            order: Vec::new(),
            vis: Vec::new(),
            tab_w: MAX_W,
            frozen_w: None,
            keep_order_until: now,
            mouse: (-1.0, -1.0),
            hover: Hit::None,
            pressed: None,
            hover_fade: HashMap::new(),
            drag: None,
            last_click: (now, 0.0, 0.0, 0),
            progress: 0.0,
            progress_alpha: 0.0,
            was_loading: false,
            load_morph: 0.0,
            spin: 0.0,
            icons: HashMap::new(),
            icon_requests: Vec::new(),
            editing: false,
            editor,
            selecting: false,
            scroll_x: 0.0,
            typed: String::new(),
            sugg: Vec::new(),
            sidx: 0,
            blink_from: now,
            pretty: None,
            placeholder,
            clipboard: None,
            hover_tab: None,
            card_shown: false,
            card_leave: None,
            last_frame: now,
            cmds: Vec::new(),
        }
    }

    // ---------- outside updates ----------

    pub fn set_theme(&mut self, dark: bool, brand: Brand) {
        self.theme = Theme::for_brand(dark, brand);
        self.model.brand = brand;
        self.pretty = None;
    }

    pub fn set_size(&mut self, scale: f32, width: f32) {
        if (scale - self.s).abs() > f32::EPSILON {
            self.s = scale;
            for v in &mut self.vis {
                v.title = None;
            }
            self.pretty = None;
            self.placeholder = self.fonts.line("Search or type a URL", URL_FONT, Weight::NORMAL, scale);
            let text = self.text();
            self.editor = Editor::new(self.fonts.line("", URL_FONT, Weight::NORMAL, scale));
            self.set_text(&text, false);
        }
        self.width = width;
        self.frozen_w = None;
        self.layout();
    }

    pub fn set_model(&mut self, model: Model) {
        let incoming: Vec<TabId> = model.tabs.iter().map(|t| t.id).collect();
        for t in &model.tabs {
            if !t.favicon.is_empty() && t.favicon != BRAND_ICON && !self.icons.contains_key(&t.favicon) {
                self.icons.insert(t.favicon.clone(), None);
                self.icon_requests.push(t.favicon.clone());
            }
        }
        // Titles changed: drop cached layouts.
        for v in &mut self.vis {
            if let Some(t) = model.tabs.iter().find(|t| t.id == v.id)
                && v.title.as_ref().is_some_and(|(s, _)| *s != t.title)
            {
                v.title = None;
            }
        }
        if !self.editing && model.url != self.model.url {
            self.pretty = None;
        }
        let dragging = self.drag.as_ref().is_some_and(|d| d.active);
        if dragging || Instant::now() < self.keep_order_until {
            self.order.retain(|id| incoming.contains(id));
            for id in &incoming {
                if !self.order.contains(id) {
                    self.order.push(*id);
                }
            }
        } else {
            self.order = incoming.clone();
        }
        for v in &mut self.vis {
            if !incoming.contains(&v.id) && !v.leaving {
                v.leaving = true;
            }
        }
        let mut fresh = false;
        for (i, id) in self.order.iter().enumerate() {
            if !self.vis.iter().any(|v| v.id == *id) {
                // New tabs start collapsed at their slot and grow in place.
                let x = self.slot_x(i);
                self.vis.push(TabVis { id: *id, x, w: 0.0, alpha: 0.0, hover: 0.0, leaving: false, title: None });
                fresh = true;
            }
        }
        if fresh {
            self.frozen_w = None;
        }
        if model.loading && !self.was_loading {
            self.progress = 0.0;
            self.progress_alpha = 1.0;
        }
        self.was_loading = model.loading;
        self.model = model;
        self.layout();
    }

    pub fn icon_loaded(&mut self, url: String, img: Option<Pixmap>) {
        self.icons.insert(url, img);
    }

    pub fn take_icon_requests(&mut self) -> Vec<String> {
        std::mem::take(&mut self.icon_requests)
    }

    pub fn set_suggestions(&mut self, items: Vec<String>) {
        self.sugg = items;
        self.sidx = 0;
    }

    /// Where the history dropdown hangs from (right edge, top), in logical pixels.
    pub fn history_anchor(&self) -> (f32, f32) {
        let r = self.geo().hist;
        (r.x + r.w, r.y + r.h + 4.0)
    }

    /// Where the bookmarks dropdown hangs from (right edge, top), in logical pixels.
    pub fn bookmarks_anchor(&self) -> (f32, f32) {
        let r = self.geo().bookmarks;
        (r.x + r.w, r.y + r.h + 4.0)
    }

    /// Where the theme dropdown hangs from (right edge, top), in logical pixels.
    pub fn theme_anchor(&self) -> (f32, f32) {
        let r = self.geo().theme;
        (r.x + r.w, r.y + r.h + 4.0)
    }

    /// The text caret in logical window coordinates, for placing the IME candidate window.
    pub fn caret(&mut self) -> Option<(f32, f32)> {
        if !self.editing {
            return None;
        }
        let (cx, _) = self.editor.cursor_position()?;
        let o = self.geo().omni;
        Some((o.x + 34.0 + cx as f32 / self.s - self.scroll_x, o.y + o.h - 6.0))
    }

    pub fn focus_address(&mut self) {
        if !self.editing {
            self.editing = true;
            let url = self.model.url.clone();
            self.set_text(&url, true);
            self.typed = url;
            self.sugg.clear();
            self.blink_from = Instant::now();
        }
    }

    pub fn blur_address(&mut self) {
        if self.editing {
            self.editing = false;
            self.selecting = false;
            self.scroll_x = 0.0;
            self.sugg.clear();
            self.cmds.push(ChromeCmd::SuggestClose);
        }
    }

    // ---------- geometry ----------

    fn snap(&self, v: f32) -> f32 {
        (v * self.s).round() / self.s
    }

    fn slot_x(&self, i: usize) -> f32 {
        self.snap(PAD + i as f32 * self.tab_w)
    }

    fn live_ids(&self) -> Vec<TabId> {
        self.order.iter().copied().filter(|id| self.vis.iter().any(|v| v.id == *id && !v.leaving)).collect()
    }

    /// Recomputes the tab width; `tick` eases each tab toward its slot.
    fn layout(&mut self) {
        let ids = self.live_ids();
        let n = ids.len().max(1) as f32;
        let avail = self.width - 3.0 * WIN_BTN_W - 38.0 - 40.0 - PAD * 2.0;
        self.tab_w = self.frozen_w.unwrap_or_else(|| (avail / n).floor().clamp(MIN_W, MAX_W));
    }

    fn slots(&self) -> Vec<TabId> {
        let mut ids = self.live_ids();
        if let Some(d) = self.drag.as_ref().filter(|d| d.active) {
            ids.retain(|id| *id != d.id);
            ids.insert(d.to.min(ids.len()), d.id);
        }
        ids
    }

    fn tabs_end(&self) -> f32 {
        self.vis.iter().map(|v| v.x + v.w).fold(PAD, f32::max)
    }

    fn geo(&self) -> Geo {
        let w = self.width;
        let dev = R { x: w - 8.0 - BTN, y: ROW_Y, w: BTN, h: BTN };
        let theme = R { x: dev.x - 2.0 - BTN, ..dev };
        let hist = R { x: theme.x - 2.0 - BTN, ..dev };
        let pip = R { x: hist.x - 2.0 - BTN, ..dev };
        let bookmarks = R { x: pip.x - 2.0 - BTN, ..dev };
        let star = R { x: bookmarks.x - 2.0 - BTN, ..dev };
        let ox = 8.0 + 3.0 * (BTN + 2.0) + 6.0;
        Geo {
            back: R { x: 8.0, y: ROW_Y, w: BTN, h: BTN },
            fwd: R { x: 8.0 + BTN + 2.0, y: ROW_Y, w: BTN, h: BTN },
            reload: R { x: 8.0 + 2.0 * (BTN + 2.0), y: ROW_Y, w: BTN, h: BTN },
            omni: R { x: ox, y: ROW_Y, w: (star.x - 8.0 - ox).max(60.0), h: BTN },
            star,
            bookmarks,
            pip,
            hist,
            theme,
            dev,
            new_tab: R { x: self.tabs_end() + 4.0, y: 5.0, w: 30.0, h: 30.0 },
            min: R { x: w - 3.0 * WIN_BTN_W, y: 0.0, w: WIN_BTN_W, h: TITLE_H },
            max: R { x: w - 2.0 * WIN_BTN_W, y: 0.0, w: WIN_BTN_W, h: TITLE_H },
            close: R { x: w - WIN_BTN_W, y: 0.0, w: WIN_BTN_W, h: TITLE_H },
        }
    }

    fn close_rect(&self, v: &TabVis) -> R {
        R { x: v.x + v.w - 7.0 - 22.0, y: TAB_Y + 6.0, w: 22.0, h: 22.0 }
    }

    fn hit(&self, x: f32, y: f32) -> Hit {
        let g = self.geo();
        if y < TITLE_H {
            for (r, h) in [(g.min, Hit::Min), (g.max, Hit::Max), (g.close, Hit::Close)] {
                if r.has(x, y) {
                    return h;
                }
            }
            if y >= TAB_Y {
                for v in self.vis.iter().filter(|v| !v.leaving) {
                    if x >= v.x && x < v.x + v.w {
                        let shows_close = v.w >= NARROW_W || v.id == self.model.active;
                        if shows_close && self.close_rect(v).has(x, y) {
                            return Hit::TabClose(v.id);
                        }
                        return Hit::Tab(v.id);
                    }
                }
            }
            if g.new_tab.has(x, y) {
                return Hit::NewTab;
            }
            return Hit::DragArea;
        }
        if y < HEIGHT {
            for (r, h) in [
                (g.back, Hit::Back),
                (g.fwd, Hit::Forward),
                (g.reload, Hit::Reload),
                (g.omni, Hit::Omni),
                (g.star, Hit::Star),
                (g.bookmarks, Hit::Bookmarks),
                (g.pip, Hit::Pip),
                (g.hist, Hit::History),
                (g.theme, Hit::Theme),
                (g.dev, Hit::Devtools),
            ] {
                if r.has(x, y) {
                    return h;
                }
            }
        }
        Hit::None
    }

    // ---------- mouse ----------

    /// True while the toolbar owns the pointer (dragging a tab or selecting text), even outside it.
    pub fn captures_mouse(&self) -> bool {
        self.drag.is_some() || self.selecting || self.pressed.is_some()
    }

    pub fn mouse_move(&mut self, x: f32, y: f32) -> CursorIcon {
        self.mouse = (x, y);
        if self.selecting {
            let (ax, ay) = self.editor_point(x);
            self.editor.action(&mut self.fonts.system, Action::Drag { x: ax, y: ay });
            return CursorIcon::Text;
        }
        if let Some(d) = &mut self.drag {
            if !d.active && (x - d.start_x).abs() >= 5.0 {
                d.active = true;
                self.end_hover_card();
            }
            if let Some(d) = self.drag.as_ref().filter(|d| d.active) {
                let n = self.live_ids().len();
                let tx = (x - d.grab_dx).clamp(PAD, PAD + (n.max(1) - 1) as f32 * self.tab_w);
                let to = (((tx - PAD) / self.tab_w).round().max(0.0) as usize).min(n.saturating_sub(1));
                let id = d.id;
                if let Some(v) = self.vis.iter_mut().find(|v| v.id == id) {
                    v.x = tx;
                }
                if let Some(d) = &mut self.drag {
                    d.to = to;
                }
            }
            return CursorIcon::Default;
        }
        self.hover = self.hit(x, y);
        if y >= TITLE_H && self.frozen_w.is_some() {
            // Closing tabs with the mouse froze their widths; release once the pointer leaves the strip.
            self.frozen_w = None;
            self.layout();
        }
        match self.hover {
            Hit::Omni => CursorIcon::Text,
            _ => CursorIcon::Default,
        }
    }

    pub fn mouse_left(&mut self) {
        if !self.captures_mouse() {
            self.hover = Hit::None;
            self.mouse = (-1.0, -1.0);
            if self.frozen_w.take().is_some() {
                self.layout();
            }
        }
    }

    fn click_count(&mut self, x: f32, y: f32) -> u32 {
        let (t, lx, ly, n) = self.last_click;
        let n = if t.elapsed() < Duration::from_millis(450) && (x - lx).abs() < 4.0 && (y - ly).abs() < 4.0 { n + 1 } else { 1 };
        self.last_click = (Instant::now(), x, y, n);
        n
    }

    fn editor_point(&self, x: f32) -> (i32, i32) {
        let o = self.geo().omni;
        let lx = (x - (o.x + 34.0) + self.scroll_x) * self.s;
        (lx.round() as i32, (URL_FONT * 0.7 * self.s) as i32)
    }

    pub fn mouse_down(&mut self, button: MouseButton, x: f32, y: f32) {
        self.mouse = (x, y);
        let hit = self.hit(x, y);
        self.cmds.push(ChromeCmd::Dismiss);
        self.end_hover_card();
        if self.editing && hit != Hit::Omni {
            self.blur_address();
        }
        match (button, hit) {
            (MouseButton::Middle, Hit::Tab(id) | Hit::TabClose(id)) => self.close_tab(id),
            (MouseButton::Right, Hit::Tab(id) | Hit::TabClose(id)) => {
                self.cmds.push(ChromeCmd::TabMenu { id, x: x as f64, y: y as f64 });
            }
            (MouseButton::Left, Hit::Tab(id)) => {
                self.cmds.push(ChromeCmd::Switch { id });
                let from = self.order.iter().position(|t| *t == id).unwrap_or(0);
                let tab_x = self.vis.iter().find(|v| v.id == id).map_or(x, |v| v.x);
                self.drag = Some(Drag { id, start_x: x, grab_dx: x - tab_x, from, to: from, active: false });
            }
            (MouseButton::Left, Hit::DragArea) => {
                let n = self.click_count(x, y);
                self.cmds.push(if n >= 2 { ChromeCmd::Maximize } else { ChromeCmd::Drag });
            }
            (MouseButton::Left, Hit::Omni) => {
                let n = self.click_count(x, y);
                self.cmds.push(ChromeCmd::FocusChrome);
                if !self.editing {
                    self.focus_address();
                } else {
                    let (ax, ay) = self.editor_point(x);
                    let action = match n {
                        1 => Action::Click { x: ax, y: ay },
                        2 => Action::DoubleClick { x: ax, y: ay },
                        _ => Action::TripleClick { x: ax, y: ay },
                    };
                    self.editor.action(&mut self.fonts.system, action);
                    self.selecting = true;
                    self.blink_from = Instant::now();
                }
            }
            (MouseButton::Left, h) if h != Hit::None => self.pressed = Some(h),
            _ => {}
        }
    }

    pub fn mouse_up(&mut self, button: MouseButton, x: f32, y: f32) {
        if button != MouseButton::Left {
            return;
        }
        self.selecting = false;
        if let Some(d) = self.drag.take()
            && d.active
        {
            self.order.retain(|id| *id != d.id);
            self.order.insert(d.to.min(self.order.len()), d.id);
            if d.to != d.from {
                self.keep_order_until = Instant::now() + Duration::from_millis(500);
                self.cmds.push(ChromeCmd::MoveTab { id: d.id, to: d.to });
            }
        }
        let Some(p) = self.pressed.take() else { return };
        if self.hit(x, y) != p {
            return;
        }
        let cmd = match p {
            Hit::TabClose(id) => {
                self.close_tab(id);
                return;
            }
            Hit::NewTab => ChromeCmd::NewTab,
            Hit::Min => ChromeCmd::Minimize,
            Hit::Max => ChromeCmd::Maximize,
            Hit::Close => ChromeCmd::CloseWindow,
            Hit::Back if self.model.can_back => ChromeCmd::Back,
            Hit::Forward if self.model.can_forward => ChromeCmd::Forward,
            Hit::Reload => {
                if self.model.loading {
                    ChromeCmd::Stop
                } else {
                    ChromeCmd::Reload
                }
            }
            Hit::Star if !self.model.url.is_empty() => ChromeCmd::ToggleBookmark,
            Hit::Bookmarks => {
                let (x, y) = self.bookmarks_anchor();
                ChromeCmd::BookmarksMenu { x: x as f64, y: y as f64 }
            }
            Hit::Pip => ChromeCmd::Pip,
            Hit::History => {
                let (x, y) = self.history_anchor();
                ChromeCmd::HistoryMenu { x: x as f64, y: y as f64 }
            }
            Hit::Theme => {
                let (x, y) = self.theme_anchor();
                ChromeCmd::ThemeMenu { x: x as f64, y: y as f64 }
            }
            Hit::Devtools => ChromeCmd::Devtools,
            _ => return,
        };
        self.cmds.push(cmd);
    }

    fn close_tab(&mut self, id: TabId) {
        // Keep widths until the pointer leaves the strip, so the next close button lands under it.
        self.frozen_w = Some(self.tab_w);
        if let Some(v) = self.vis.iter_mut().find(|v| v.id == id) {
            v.leaving = true;
        }
        self.order.retain(|t| *t != id);
        self.cmds.push(ChromeCmd::CloseTab { id });
    }

    // ---------- hover cards ----------

    fn end_hover_card(&mut self) {
        self.hover_tab = None;
        self.card_leave = None;
        if self.card_shown {
            self.card_shown = false;
            self.cmds.push(ChromeCmd::TabHoverEnd);
        }
    }

    fn update_hover_card(&mut self, now: Instant) {
        let over = match self.hover {
            Hit::Tab(id) | Hit::TabClose(id) if self.drag.as_ref().is_none_or(|d| !d.active) => Some(id),
            _ => None,
        };
        match (over, self.hover_tab) {
            (Some(id), Some((cur, _, _))) if id == cur => {}
            (Some(id), _) => {
                self.hover_tab = Some((id, now, false));
                self.card_leave = None;
            }
            (None, Some(_)) => {
                self.hover_tab = None;
                self.card_leave = Some(now + Duration::from_millis(150));
            }
            (None, None) => {}
        }
        if let Some((id, since, fired)) = self.hover_tab {
            let delay = if self.card_shown { Duration::ZERO } else { Duration::from_millis(500) };
            if !fired && now >= since + delay {
                let x = self.vis.iter().find(|v| v.id == id).map_or(PAD, |v| v.x);
                self.cmds.push(ChromeCmd::TabHover { id, x: x as f64, y: (TITLE_H + 2.0) as f64 });
                self.card_shown = true;
                self.hover_tab = Some((id, since, true));
            }
        }
        if let Some(t) = self.card_leave
            && now >= t
        {
            self.card_leave = None;
            if self.card_shown {
                self.card_shown = false;
                self.cmds.push(ChromeCmd::TabHoverEnd);
            }
        }
    }

    // ---------- address bar text ----------

    fn text(&self) -> String {
        self.editor.with_buffer(|b| b.lines.first().map(|l| l.text().to_owned()).unwrap_or_default())
    }

    fn set_text(&mut self, text: &str, select_all: bool) {
        let one_line: String = text.chars().filter(|c| !c.is_control()).collect();
        self.editor.with_buffer_mut(|b| b.set_text(&one_line, &attrs(Weight::NORMAL), Shaping::Advanced, None));
        self.editor.set_cursor(Cursor::new(0, one_line.len()));
        self.editor.set_selection(if select_all { Selection::Normal(Cursor::new(0, 0)) } else { Selection::None });
        self.scroll_x = 0.0;
    }

    fn query(&mut self) {
        let text = self.text();
        self.typed = text.clone();
        self.sidx = 0;
        let o = self.geo().omni;
        self.cmds.push(ChromeCmd::Query { text, x: o.x as f64, y: (o.y + o.h + 4.0) as f64, w: o.w as f64 });
    }

    fn insert(&mut self, s: &str) {
        let clean: String = s.chars().filter(|c| !c.is_control()).collect();
        if clean.is_empty() {
            return;
        }
        self.editor.delete_selection();
        self.editor.insert_string(&clean, None);
        self.query();
    }

    fn clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut()
    }

    /// Moves the caret, extending the selection when Shift is held.
    fn motion(&mut self, m: Motion, shift: bool) {
        let sel = self.editor.selection();
        if shift {
            if sel == Selection::None {
                self.editor.set_selection(Selection::Normal(self.editor.cursor()));
            }
        } else if let Some((a, b)) = self.editor.selection_bounds() {
            // Without Shift, Left/Right just collapse the selection to its edge.
            self.editor.set_selection(Selection::None);
            match m {
                Motion::Left | Motion::LeftWord => return self.editor.set_cursor(a),
                Motion::Right | Motion::RightWord => return self.editor.set_cursor(b),
                _ => {}
            }
        }
        self.editor.action(&mut self.fonts.system, Action::Motion(m));
    }

    fn select_all(&mut self) {
        let len = self.text().len();
        self.editor.set_selection(Selection::Normal(Cursor::new(0, 0)));
        self.editor.set_cursor(Cursor::new(0, len));
    }

    pub fn ime(&mut self, text: &str) {
        if self.editing {
            self.insert(text);
            self.blink_from = Instant::now();
        }
    }

    /// Handles a key press while the browser window (not a page) has keyboard focus.
    pub fn key(&mut self, e: &KeyEvent, mods: ModifiersState) {
        if e.state != ElementState::Pressed {
            return;
        }
        let (ctrl, shift, alt) = (mods.control_key(), mods.shift_key(), mods.alt_key());
        let ch = match &e.logical_key {
            Key::Character(c) => Some(c.to_lowercase()),
            _ => None,
        };
        // Browser shortcuts, whether or not the address bar is being edited.
        let shortcut = match (&e.logical_key, ch.as_deref()) {
            (_, Some("t")) if ctrl => Some(ChromeCmd::NewTab),
            (_, Some("w")) if ctrl => Some(ChromeCmd::CloseTab { id: self.model.active }),
            (_, Some("r")) if ctrl => Some(ChromeCmd::Reload),
            (Key::F5, _) => Some(ChromeCmd::Reload),
            (Key::F12, _) => Some(ChromeCmd::Devtools),
            (_, Some("i")) if ctrl && shift => Some(ChromeCmd::Devtools),
            (_, Some("d")) if ctrl && !shift && !alt => Some(ChromeCmd::ToggleBookmark),
            (_, Some("b")) if ctrl && !shift && !alt => {
                let (x, y) = self.bookmarks_anchor();
                Some(ChromeCmd::BookmarksMenu { x: x as f64, y: y as f64 })
            }
            (_, Some("p")) if alt && !ctrl && !shift => Some(ChromeCmd::Pip),
            (_, Some("h")) if ctrl => {
                let (x, y) = self.history_anchor();
                Some(ChromeCmd::HistoryMenu { x: x as f64, y: y as f64 })
            }
            (Key::Tab, _) if ctrl => Some(if shift { ChromeCmd::PrevTab } else { ChromeCmd::NextTab }),
            (Key::ArrowLeft, _) if alt => Some(ChromeCmd::Back),
            (Key::ArrowRight, _) if alt => Some(ChromeCmd::Forward),
            _ => None,
        };
        if let Some(cmd) = shortcut {
            if let ChromeCmd::CloseTab { id } = cmd {
                self.close_tab(id);
            } else {
                self.cmds.push(cmd);
            }
            return;
        }
        if ctrl && ch.as_deref() == Some("l") {
            self.focus_address();
            self.select_all();
            self.cmds.push(ChromeCmd::FocusChrome);
            return;
        }
        if !self.editing {
            if e.logical_key == Key::Escape {
                self.cmds.push(ChromeCmd::Dismiss);
            }
            return;
        }
        self.blink_from = Instant::now();
        match &e.logical_key {
            Key::Enter => {
                let input = self.text();
                if !input.trim().is_empty() {
                    self.blur_address();
                    self.cmds.push(ChromeCmd::Navigate { input });
                }
            }
            Key::Escape => {
                if self.sugg.is_empty() {
                    self.blur_address();
                    self.cmds.push(ChromeCmd::FocusPage);
                } else {
                    let typed = self.typed.clone();
                    self.set_text(&typed, false);
                    self.sugg.clear();
                    self.cmds.push(ChromeCmd::SuggestClose);
                }
            }
            Key::ArrowDown | Key::ArrowUp if !self.sugg.is_empty() => {
                let n = self.sugg.len();
                self.sidx = if e.logical_key == Key::ArrowDown { (self.sidx + 1) % n } else { (self.sidx + n - 1) % n };
                let text = if self.sidx == 0 { self.typed.clone() } else { self.sugg[self.sidx].clone() };
                self.set_text(&text, false);
                self.cmds.push(ChromeCmd::SuggestSelect { index: self.sidx });
            }
            Key::ArrowLeft => self.motion(if ctrl { Motion::LeftWord } else { Motion::Left }, shift),
            Key::ArrowRight => self.motion(if ctrl { Motion::RightWord } else { Motion::Right }, shift),
            Key::Home => self.motion(Motion::Home, shift),
            Key::End => self.motion(Motion::End, shift),
            Key::Backspace | Key::Delete => {
                if ctrl && self.editor.selection() == Selection::None {
                    self.editor.set_selection(Selection::Normal(self.editor.cursor()));
                    let m = if e.logical_key == Key::Backspace { Motion::LeftWord } else { Motion::RightWord };
                    self.editor.action(&mut self.fonts.system, Action::Motion(m));
                }
                let a = if e.logical_key == Key::Backspace { Action::Backspace } else { Action::Delete };
                self.editor.action(&mut self.fonts.system, a);
                self.query();
            }
            Key::Character(_) if ctrl => match ch.as_deref() {
                Some("a") => self.select_all(),
                Some("c") | Some("x") => {
                    if let Some(sel) = self.editor.copy_selection() {
                        if let Some(cb) = self.clipboard() {
                            let _ = cb.set_text(sel);
                        }
                        if ch.as_deref() == Some("x") {
                            self.editor.delete_selection();
                            self.query();
                        }
                    }
                }
                Some("v") => {
                    if let Some(text) = self.clipboard().and_then(|cb| cb.get_text().ok()) {
                        self.insert(&text.replace(['\r', '\n'], " "));
                    }
                }
                _ => {}
            },
            _ if !ctrl && !alt => {
                if let Some(t) = e.text {
                    self.insert(t);
                }
            }
            _ => {}
        }
    }

    // ---------- animation ----------

    /// Advances animations and timers. Returns when the next frame is needed (`None` = idle).
    pub fn tick(&mut self, now: Instant) -> Option<Instant> {
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;
        self.update_hover_card(now);
        // Rates tuned so motion settles in ~0.2s and hover fades in ~0.12s.
        let k = 1.0 - (-dt * 26.0).exp();
        let kf = 1.0 - (-dt * 30.0).exp();
        let mut busy = false;

        self.layout();
        let slots = self.slots();
        let drag_id = self.drag.as_ref().filter(|d| d.active).map(|d| d.id);
        let tab_w = self.tab_w;
        let targets: Vec<(TabId, f32)> = slots.iter().enumerate().map(|(i, id)| (*id, self.slot_x(i))).collect();
        let (hover, active) = (self.hover, self.model.active);
        for v in &mut self.vis {
            let hovered = matches!(hover, Hit::Tab(id) | Hit::TabClose(id) if id == v.id);
            busy |= approach01(&mut v.hover, if hovered && v.id != active { 1.0 } else { 0.0 }, kf);
            if v.leaving {
                busy |= approach(&mut v.w, 0.0, k);
                busy |= approach01(&mut v.alpha, 0.0, kf);
                continue;
            }
            busy |= approach(&mut v.w, tab_w, k);
            busy |= approach01(&mut v.alpha, 1.0, kf);
            if Some(v.id) != drag_id
                && let Some((_, tx)) = targets.iter().find(|(id, _)| *id == v.id)
            {
                busy |= approach(&mut v.x, *tx, k);
            }
        }
        self.vis.retain(|v| !(v.leaving && v.w < 0.5));
        busy |= drag_id.is_some();

        for h in [
            Hit::NewTab, Hit::Min, Hit::Max, Hit::Close, Hit::Back, Hit::Forward, Hit::Reload,
            Hit::Omni, Hit::Star, Hit::Bookmarks, Hit::Pip, Hit::History, Hit::Theme, Hit::Devtools,
        ] {
            let target = if self.hover == h { 1.0 } else { 0.0 };
            let f = self.hover_fade.entry(hit_key(h)).or_insert(0.0);
            busy |= approach01(f, target, kf);
        }

        busy |= approach01(&mut self.load_morph, if self.model.loading { 1.0 } else { 0.0 }, kf);
        if self.model.loading {
            // Eases toward 90% over a few seconds, then waits for the load to finish.
            self.progress += (0.9 - self.progress) * (1.0 - (-dt / 2.5).exp());
            busy = true;
        } else if self.progress_alpha > 0.0 {
            busy |= approach01(&mut self.progress, 1.0, 1.0 - (-dt * 18.0).exp());
            if self.progress > 0.97 {
                self.progress_alpha = (self.progress_alpha - dt * 4.0).max(0.0);
            }
            busy |= self.progress_alpha > 0.0;
        }
        if self.model.tabs.iter().any(|t| t.loading) {
            self.spin = (self.spin + dt / 0.65).fract();
            busy = true;
        }

        let mut next: Option<Instant> = busy.then(|| now + crate::pacer::FRAME);
        let mut wake = |t: Instant| next = Some(next.map_or(t, |n| n.min(t)));
        if let Some((_, since, false)) = self.hover_tab {
            wake(since + if self.card_shown { Duration::ZERO } else { Duration::from_millis(500) });
        }
        if let Some(t) = self.card_leave {
            wake(t);
        }
        if self.editing {
            // Caret blink.
            let phase = now.saturating_duration_since(self.blink_from).as_millis() % 530;
            wake(now + Duration::from_millis((530 - phase) as u64));
        }
        next
    }

    // ---------- drawing ----------

    pub fn render(&mut self, pm: &mut Pixmap) {
        let s = self.s;
        let th = self.theme;
        let mut c = Canvas { pm, s };
        c.rect(0.0, 0.0, self.width, TITLE_H, th.strip);
        c.rect(0.0, TITLE_H, self.width, HEIGHT - TITLE_H, th.toolbar);

        self.draw_tabs(&mut c, &th);
        let g = self.geo();

        // New tab button.
        self.button_bg(&mut c, g.new_tab, Hit::NewTab, 8.0, &th);
        c.icon(Icon::Plus, g.new_tab.x + 7.0, g.new_tab.y + 7.0, 16.0, th.dim.mix(th.fg, self.fade(Hit::NewTab)));

        // Window controls.
        for (r, h, icon) in [
            (g.min, Hit::Min, Icon::WinMin),
            (g.max, Hit::Max, if self.model.maximized { Icon::WinRestore } else { Icon::WinMax }),
            (g.close, Hit::Close, Icon::WinClose),
        ] {
            let f = self.fade(h);
            let (bg, fg) = if h == Hit::Close {
                (th.close.with_alpha(f), th.fg.mix(Rgba(255, 255, 255, 255), f))
            } else {
                (th.btn_hover.with_alpha(f), th.fg)
            };
            c.rect(r.x, r.y, r.w, r.h, bg);
            c.icon(icon, r.x + (r.w - 10.0) / 2.0, r.y + (r.h - 10.0) / 2.0, 10.0, fg);
        }

        // Toolbar buttons.
        let disabled = |on: bool| if on { 1.0 } else { 0.3 };
        for (r, h, icon, on) in [
            (g.back, Hit::Back, Icon::Back, self.model.can_back),
            (g.fwd, Hit::Forward, Icon::Forward, self.model.can_forward),
            (g.bookmarks, Hit::Bookmarks, Icon::Bookmarks, true),
            (g.hist, Hit::History, Icon::History, true),
        ] {
            if on {
                self.button_bg(&mut c, r, h, 9.0, &th);
            }
            c.icon(icon, r.x + 9.0, r.y + 9.0, 16.0, th.fg.with_alpha(disabled(on)));
        }
        // Star (bookmark current page) button.
        let can_star = !self.model.url.is_empty();
        if self.model.bookmarked {
            c.rrect(g.star.x, g.star.y, g.star.w, g.star.h, [9.0; 4], th.accent.with_alpha(0.14));
        } else if can_star {
            self.button_bg(&mut c, g.star, Hit::Star, 9.0, &th);
        }
        c.icon(
            if self.model.bookmarked { Icon::StarFilled } else { Icon::Star },
            g.star.x + 9.0,
            g.star.y + 9.0,
            16.0,
            if self.model.bookmarked { th.accent } else { th.fg.with_alpha(disabled(can_star)) },
        );
        self.button_bg(&mut c, g.reload, Hit::Reload, 9.0, &th);
        let m = self.load_morph;
        c.icon(Icon::Reload, g.reload.x + 9.0, g.reload.y + 9.0, 16.0, th.fg.with_alpha(1.0 - m));
        c.icon(Icon::Stop, g.reload.x + 9.0, g.reload.y + 9.0, 16.0, th.fg.with_alpha(m));
        for (r, h, icon, active) in [
            (g.pip, Hit::Pip, Icon::Pip, self.model.pip),
            (g.theme, Hit::Theme, Icon::Palette, self.model.brand == Brand::Uzaki),
            (g.dev, Hit::Devtools, Icon::Code, self.model.devtools),
        ] {
            if active {
                c.rrect(r.x, r.y, r.w, r.h, [9.0; 4], th.accent.with_alpha(0.14));
            } else {
                self.button_bg(&mut c, r, h, 9.0, &th);
            }
            c.icon(icon, r.x + 9.0, r.y + 9.0, 16.0, if active { th.accent } else { th.fg });
        }

        self.draw_omni(&mut c, g.omni, &th);

        // Page-load progress along the bottom edge, then the hairline under the toolbar.
        c.hairline_h(0.0, HEIGHT - 1.0 / s, self.width, th.border);
        if self.progress_alpha > 0.0 {
            let w = self.width * self.progress;
            c.rect(0.0, HEIGHT - 2.0, w, 2.0, th.accent.mix(th.accent2, 0.35).with_alpha(self.progress_alpha));
        }
    }

    fn fade(&self, h: Hit) -> f32 {
        self.hover_fade.get(&hit_key(h)).copied().unwrap_or(0.0)
    }

    fn button_bg(&self, c: &mut Canvas, r: R, h: Hit, radius: f32, th: &Theme) {
        let pressed = self.pressed == Some(h) && self.hover == h;
        let col = if pressed { th.btn_press } else { th.btn_hover.with_alpha(self.fade(h)) };
        if col.3 > 0 {
            c.rrect(r.x, r.y, r.w, r.h, [radius; 4], col);
        }
    }

    fn draw_tabs(&mut self, c: &mut Canvas, th: &Theme) {
        let slots = self.slots();
        let active = self.model.active;
        let drag_id = self.drag.as_ref().filter(|d| d.active).map(|d| d.id);
        let active_at = slots.iter().position(|id| *id == active);
        let hover_at = match self.hover {
            Hit::Tab(id) | Hit::TabClose(id) => slots.iter().position(|t| *t == id),
            _ => None,
        };
        // Leaving tabs underneath, then normal ones, then the active tab, then the dragged one on top.
        let mut idx: Vec<usize> = (0..self.vis.len()).collect();
        idx.sort_by_key(|&i| {
            let v = &self.vis[i];
            (v.leaving as u8 == 0, v.id == active, Some(v.id) == drag_id)
        });
        for i in idx {
            let (id, x, w, alpha, hover, leaving) = {
                let v = &self.vis[i];
                (v.id, self.snap(v.x), self.snap(v.x + v.w) - self.snap(v.x), v.alpha, v.hover, v.leaving)
            };
            if w < 1.0 || alpha <= 0.01 {
                continue;
            }
            let is_active = id == active && !leaving;
            let dragging = Some(id) == drag_id;
            let slot = slots.iter().position(|t| *t == id);
            if dragging {
                for (spread, a) in [(6.0, 0.06), (3.0, 0.1), (1.0, 0.14)] {
                    c.rrect(x - spread, TAB_Y - spread * 0.3, w + spread * 2.0, TAB_H + spread, [10.0 + spread; 4], Rgba(0, 0, 0, 255).with_alpha(a));
                }
                c.rrect(x, TAB_Y, w, TAB_H, [10.0; 4], th.toolbar);
            } else if is_active {
                c.rrect(x, TAB_Y, w, TAB_H, [10.0, 10.0, 0.0, 0.0], th.toolbar);
                c.foot(x - 10.0, TITLE_H, 10.0, false, th.toolbar);
                c.foot(x + w, TITLE_H, 10.0, true, th.toolbar);
            } else if hover > 0.0 {
                c.rrect(x, TAB_Y, w, TAB_H, [10.0, 10.0, 0.0, 0.0], th.tab_hover.with_alpha(hover * alpha));
            }
            // Separator after inactive tabs, except next to the active or hovered tab and after the last.
            if let Some(i) = slot
                && !dragging
                && !leaving
            {
                let near = |at: Option<usize>| at.is_some_and(|a| i == a || i + 1 == a);
                if !near(active_at) && !near(hover_at) && i + 1 < slots.len() {
                    c.hairline_v(x + w, TAB_Y + 10.0, TAB_H - 20.0, th.border.with_alpha(alpha));
                }
            }
            self.draw_tab_content(c, i, x, w, alpha, is_active || dragging, th);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_tab_content(&mut self, c: &mut Canvas, i: usize, x: f32, w: f32, alpha: f32, active: bool, th: &Theme) {
        let id = self.vis[i].id;
        let Some(info) = self.model.tabs.iter().find(|t| t.id == id) else { return };
        let (title, favicon, loading, pip) = (info.title.clone(), info.favicon.clone(), info.loading, info.pip);
        let hovered = matches!(self.hover, Hit::Tab(h) | Hit::TabClose(h) if h == id);
        let narrow = w < NARROW_W;
        let fg = if active || hovered { th.fg } else { th.dim }.with_alpha(alpha);
        let cy = TAB_Y + TAB_H / 2.0;
        let icon_x = if narrow && !active { x + (w - 16.0) / 2.0 } else { x + 12.0 };
        let show_close = !self.vis[i].leaving && (active || hovered) && (!narrow || active) && w >= 44.0;
        let close = self.close_rect(&self.vis[i]);
        // Favicon, spinner, or the Cederic/Uzaki badge.
        if loading {
            c.spinner(icon_x + 8.0, cy, 6.0, self.spin, th.accent.with_alpha(alpha));
        } else if favicon == BRAND_ICON {
            c.rrect(icon_x, cy - 8.0, 16.0, 16.0, [4.0; 4], th.accent.mix(th.accent2, 0.3).with_alpha(alpha));
            let mut b = self.fonts.line(self.model.brand.badge_letter(), 11.0, Weight::BOLD, self.s);
            let bw = text_width(&b, self.s);
            c.text(&mut self.fonts, &mut b, icon_x + (16.0 - bw) / 2.0, cy - 7.5, Rgba(255, 255, 255, 255).with_alpha(alpha), f32::MAX, 0.0);
        } else if let Some(Some(img)) = self.icons.get(&favicon) {
            c.image(img, icon_x, cy - 8.0, 16.0, 16.0, alpha);
        } else {
            c.icon(Icon::Globe, icon_x, cy - 8.0, 16.0, th.faint.with_alpha(alpha));
        }
        if pip && !loading {
            let ring = if active { th.toolbar } else { th.strip };
            c.circle(icon_x + 14.5, cy + 6.5, 4.5, ring.with_alpha(alpha));
            c.circle(icon_x + 14.5, cy + 6.5, 3.0, th.accent.with_alpha(alpha));
        }
        if !narrow {
            let s = self.s;
            let text = if title.is_empty() { "New Tab".to_owned() } else { title };
            let v = &mut self.vis[i];
            if v.title.as_ref().is_none_or(|(t, _)| *t != text) {
                let b = self.fonts.line(&text, FONT, Weight::NORMAL, s);
                v.title = Some((text, b));
            }
            let clip = if show_close { close.x - 2.0 } else { x + w - 7.0 };
            if let Some((_, buf)) = &mut self.vis[i].title {
                c.text(&mut self.fonts, buf, x + 36.0, cy - FONT * 1.35 / 2.0, fg, clip, 20.0);
            }
        }
        if show_close {
            if self.hover == Hit::TabClose(id) {
                c.circle(close.x + 11.0, close.y + 11.0, 11.0, th.btn_hover);
            }
            c.icon(Icon::Close, close.x + 5.5, close.y + 5.5, 11.0, fg.with_alpha(0.75 * alpha));
        }
    }

    fn draw_omni(&mut self, c: &mut Canvas, o: R, th: &Theme) {
        let f = if self.editing { 0.0 } else { self.fade(Hit::Omni) };
        if self.editing {
            c.rrect(o.x - 3.0, o.y - 3.0, o.w + 6.0, o.h + 6.0, [20.0; 4], th.accent.with_alpha(0.22));
            c.rrect(o.x - 1.0, o.y - 1.0, o.w + 2.0, o.h + 2.0, [18.0; 4], th.accent);
        }
        c.rrect(o.x, o.y, o.w, o.h, [17.0; 4], th.field.mix(th.field_hover, f));

        let url = self.model.url.clone();
        let (icon, col) = if url.starts_with("https://") {
            (Icon::Lock, th.dim)
        } else if url.starts_with("http://") {
            (Icon::Info, th.warn)
        } else {
            (Icon::Search, th.dim)
        };
        c.icon(icon, o.x + 11.0, o.y + 10.0, 14.0, col);

        let tx = o.x + 34.0;
        let right = o.x + o.w - 14.0;
        let ty = o.y + (o.h - URL_FONT * 1.35) / 2.0;
        if self.editing {
            self.draw_editor(c, tx, ty, right, th);
            return;
        }
        if url.is_empty() {
            c.text(&mut self.fonts, &mut self.placeholder, tx, ty, th.faint, right, 0.0);
            return;
        }
        if self.pretty.as_ref().is_none_or(|(u, _)| *u != url) {
            let b = self.pretty_url(&url, th);
            self.pretty = Some((url, b));
        }
        if let Some((_, b)) = &mut self.pretty {
            c.text(&mut self.fonts, b, tx, ty, th.dim, right, 24.0);
        }
    }

    /// The URL with the scheme (for https) and "www." hidden, and the host emphasised.
    fn pretty_url(&mut self, url: &str, th: &Theme) -> Buffer {
        let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
        let rest = rest.strip_prefix("www.").unwrap_or(rest);
        let split = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (host, path) = rest.split_at(split);
        let path = if path == "/" { "" } else { path };
        let prefix = if scheme == "http" { "http://" } else { "" };
        let col = |c: Rgba| cosmic_text::Color::rgba(c.0, c.1, c.2, c.3);
        let mut b = self.fonts.line("", URL_FONT, Weight::NORMAL, self.s);
        let base = attrs(Weight::NORMAL);
        b.set_rich_text(
            [
                (prefix, base.clone().color(col(th.dim))),
                (host, base.clone().color(col(th.fg))),
                (path, base.clone().color(col(th.dim))),
            ],
            &base,
            Shaping::Advanced,
            None,
        );
        b.shape_until_scroll(&mut self.fonts.system, false);
        b
    }

    fn draw_editor(&mut self, c: &mut Canvas, x: f32, y: f32, right: f32, th: &Theme) {
        let s = self.s;
        self.editor.shape_as_needed(&mut self.fonts.system, false);
        if self.text().is_empty() {
            c.text(&mut self.fonts, &mut self.placeholder, x, y, th.faint, right, 0.0);
        }
        // Keep the caret in view by scrolling the text horizontally.
        let avail = right - x;
        if let Some((cx, _)) = self.editor.cursor_position() {
            let cx = cx as f32 / s;
            if cx - self.scroll_x > avail - 2.0 {
                self.scroll_x = cx - avail + 2.0;
            } else if cx < self.scroll_x {
                self.scroll_x = cx;
            }
        }
        let caret_on = self.blink_from.elapsed().as_millis() / 530 % 2 == 0;
        let col = |c: Rgba| cosmic_text::Color::rgba(c.0, c.1, c.2, c.3);
        let caret = if caret_on { col(th.fg) } else { col(Rgba(0, 0, 0, 0)) };
        let (ox, oy) = (((x - self.scroll_x) * s).round() as i32, (y * s).round() as i32);
        let (l, r) = ((x * s) as i32, (right * s) as i32);
        let (w, h) = (c.pm.width() as i32, c.pm.height() as i32);
        let data = c.pm.data_mut();
        self.editor.draw(
            &mut self.fonts.system,
            &mut self.fonts.cache,
            col(th.fg),
            caret,
            col(th.selection),
            col(th.fg),
            |gx, gy, gw, gh, color| {
                for py in oy + gy..oy + gy + gh as i32 {
                    for px in ox + gx..ox + gx + gw as i32 {
                        if px < l || px >= r || py < 0 || py >= h || px >= w {
                            continue;
                        }
                        crate::gfx::blend_px(data, (py * w + px) as usize, color);
                    }
                }
            },
        );
    }
}

struct Geo {
    back: R,
    fwd: R,
    reload: R,
    omni: R,
    star: R,
    bookmarks: R,
    pip: R,
    hist: R,
    theme: R,
    dev: R,
    new_tab: R,
    min: R,
    max: R,
    close: R,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders the toolbar off-screen: `CEDERIC_SHOT=<dir> cargo test -- --ignored render_toolbar`.
    #[test]
    #[ignore]
    fn render_toolbar() {
        let dir = std::path::PathBuf::from(std::env::var("CEDERIC_SHOT").expect("CEDERIC_SHOT"));
        for (name, dark, editing) in [("toolbar-dark", true, false), ("toolbar-light", false, false), ("toolbar-editing", true, true)] {
            let s = 2.0;
            let mut tb = Toolbar::new(dark, s, 1280.0);
            let tab = |id, title: &str, favicon: &str, loading| TabInfo { id, title: title.into(), favicon: favicon.into(), loading, pip: false };
            tb.set_model(Model {
                tabs: vec![
                    tab(1, "Rust Programming Language", "", false),
                    tab(2, "GitHub · Change is constant. GitHub keeps you ahead.", "", true),
                    tab(3, "rust borrow checker - Cederic Search", BRAND_ICON, false),
                ],
                active: 3,
                url: "https://search.cederic.com/search?q=rust+borrow+checker".into(),
                can_back: true,
                loading: false,
                ..Model::default()
            });
            if editing {
                tb.focus_address();
            }
            // Let the open animations settle.
            let start = Instant::now();
            for i in 1..=60 {
                tb.tick(start + Duration::from_millis(i * 16));
            }
            tb.mouse_move(420.0, 20.0);
            for i in 61..=80 {
                tb.tick(start + Duration::from_millis(i * 16));
            }
            let mut pm = Pixmap::new((1280.0 * s) as u32, (HEIGHT * s) as u32).unwrap();
            tb.render(&mut pm);
            pm.save_png(dir.join(format!("{name}.png"))).unwrap();
        }
    }

    /// Times one toolbar frame: `cargo test --release -- --ignored --nocapture bench_frame`.
    #[test]
    #[ignore]
    fn bench_frame() {
        let s = 2.0;
        let mut tb = Toolbar::new(true, s, 1280.0);
        let tabs = (1..=8)
            .map(|id| TabInfo { id, title: format!("Some page title number {id}"), favicon: String::new(), loading: false, pip: false })
            .collect();
        tb.set_model(Model { tabs, active: 3, url: "https://example.com/some/path".into(), ..Model::default() });
        let mut pm = Pixmap::new((1280.0 * s) as u32, (HEIGHT * s) as u32).unwrap();
        let start = Instant::now();
        for i in 0..120u64 {
            tb.mouse_move(100.0 + i as f32 * 8.0, 20.0);
            tb.tick(start + Duration::from_millis(i * 16));
            tb.render(&mut pm);
        }
        let per = start.elapsed() / 120;
        println!("toolbar frame (tick + render at 2x): {per:?}");
    }
}

