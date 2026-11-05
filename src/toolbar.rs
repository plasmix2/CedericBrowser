//! The native browser chrome: tab strip, window controls, toolbar buttons and the address bar,
//! drawn by the ANVL engine (`anvil.rs`) into the top of the main window. Input arrives as plain window events;
//! what the user asked for comes out as `ChromeCmd`s in `cmds`.

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use anvil_ui::{Editor, InputKind};
use tao::{
    event::{ElementState, KeyEvent, MouseButton},
    keyboard::{Key, ModifiersState},
    window::CursorIcon,
};
use tiny_skia::Pixmap;

use crate::{
    ChromeCmd, TabId,
    anvil::{self, Engine, Painter, Weight},
    gfx::{Brand, Icon, PersonaColor, Rgba, Theme},
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
/// Space between the groups of buttons on the right of the toolbar (the usual gap is 2).
const GROUP_GAP: f32 = 12.0;
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
    pub color: PersonaColor,
    pub badge_letter: String,
    /// The active persona's profile picture (a data: or web URL), or empty for none.
    pub pfp: String,
    /// A downloaded update is waiting for a restart: badge the persona button.
    pub update_ready: bool,
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
    /// The ANVL engine that draws the chrome; `None` only if the machine has no usable GPU.
    engine: Option<Engine>,
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
    hover_fade: [f32; 16],
    /// 0..1 phase of the hover gradient's slow turn.
    shimmer: f32,
    drag: Option<Drag>,
    last_click: (Instant, f32, f32, u32),
    progress: f32,
    progress_alpha: f32,
    was_loading: bool,
    load_morph: f32,
    /// 0..1 fade of the address bar's focus ring.
    focus_f: f32,
    /// 1 -> 0 after a page is bookmarked: drives the star's little pop.
    star_pop: f32,
    spin: f32,
    icons: HashMap<String, Option<Pixmap>>,
    icon_requests: Vec<String>,
    // address bar
    editing: bool,
    /// The address bar text (one line) and its caret/selection, in characters.
    buf: String,
    ed: Editor,
    selecting: bool,
    scroll_x: f32,
    typed: String,
    sugg: Vec<String>,
    sidx: usize,
    blink_from: Instant,
    clipboard: Option<arboard::Clipboard>,
    // hover cards
    hover_tab: Option<(TabId, Instant, bool)>,
    card_shown: bool,
    card_leave: Option<Instant>,
    /// Button under the pointer, when it started hovering, and whether its tooltip is up.
    tip: Option<(Hit, Instant, bool)>,
    last_frame: Instant,
    bg: Option<Pixmap>,
    /// Whether `bg` has been uploaded to the engine since it last changed.
    bg_uploaded: bool,
    logo: Option<Pixmap>,
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

const TIP_DELAY: Duration = Duration::from_millis(450);

fn hit_key(h: Hit) -> usize {
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
        let engine = Engine::new();
        if engine.is_none() {
            eprintln!("persona: no GPU adapter for the ANVL engine; the toolbar will be blank");
        }
        let now = Instant::now();
        Self {
            theme: Theme::for_brand(dark, brand),
            engine,
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
            hover_fade: [0.0; 16],
            shimmer: 0.0,
            drag: None,
            last_click: (now, 0.0, 0.0, 0),
            progress: 0.0,
            progress_alpha: 0.0,
            was_loading: false,
            load_morph: 0.0,
            focus_f: 0.0,
            star_pop: 0.0,
            spin: 0.0,
            icons: HashMap::new(),
            icon_requests: Vec::new(),
            editing: false,
            buf: String::new(),
            ed: Editor::default(),
            selecting: false,
            scroll_x: 0.0,
            typed: String::new(),
            sugg: Vec::new(),
            sidx: 0,
            blink_from: now,
            clipboard: None,
            hover_tab: None,
            card_shown: false,
            card_leave: None,
            tip: None,
            last_frame: now,
            bg: None,
            bg_uploaded: false,
            logo: Pixmap::decode_png(include_bytes!("ui/logo_64.png")).ok(),
            cmds: Vec::new(),
        }
    }

    // ---------- outside updates ----------

    #[allow(dead_code)]
    pub fn set_theme(&mut self, dark: bool, brand: Brand) {
        self.set_persona_theme(dark, PersonaColor::from(brand), brand.badge_letter());
    }

    pub fn set_persona_theme(&mut self, dark: bool, color: PersonaColor, badge_letter: &str) {
        self.theme = Theme::for_persona(dark, color);
        self.model.brand = match color {
            PersonaColor::Slate => Brand::Uzaki,
            _ => Brand::Cederic,
        };
        self.model.color = color;
        self.model.badge_letter = badge_letter.to_uppercase();
    }

    pub fn set_background(&mut self, bg: Option<Pixmap>) {
        self.bg = bg;
        self.bg_uploaded = false;
    }

    pub fn set_size(&mut self, scale: f32, width: f32) {
        if (scale - self.s).abs() > f32::EPSILON {
            self.s = scale;
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
        if !model.pfp.is_empty() && !self.icons.contains_key(&model.pfp) {
            self.icons.insert(model.pfp.clone(), None);
            self.icon_requests.push(model.pfp.clone());
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
                self.vis.push(TabVis { id: *id, x, w: 0.0, alpha: 0.0, hover: 0.0, leaving: false });
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
        if model.bookmarked && !self.model.bookmarked && model.url == self.model.url {
            self.star_pop = 1.0;
        }
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
        let cx = self.width_of_chars(self.ed.caret);
        let o = self.geo().omni;
        Some((o.x + 34.0 + cx - self.scroll_x, o.y + o.h - 6.0))
    }

    pub fn focus_address(&mut self) {
        self.editing = true;
        let url = self.model.url.clone();
        self.set_text(&url, true);
        self.typed = url;
        self.sugg.clear();
        self.blink_from = Instant::now();
    }

    pub fn is_editing(&self) -> bool {
        self.editing
    }

    /// Feeds a keystroke forwarded from a New Tab (or while the address bar is active) straight
    /// into the address bar editor so typing works immediately even before Win32 focus transfers.
    pub fn auto_key(&mut self, key: &str) {
        if !self.editing {
            self.focus_address();
        }
        self.blink_from = Instant::now();
        match key {
            "Backspace" => {
                self.ed.backspace(&mut self.buf, false);
                self.query();
            }
            "Enter" => {
                let input = self.text();
                if !input.trim().is_empty() {
                    self.blur_address();
                    self.cmds.push(ChromeCmd::Navigate { input });
                }
            }
            s if s.chars().count() == 1 => {
                self.insert(s);
            }
            _ => {}
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
        // Right side, in groups: [star bookmarks history] | [picture-in-picture dev tools] | persona.
        let theme = R { x: w - 8.0 - BTN, y: ROW_Y, w: BTN, h: BTN };
        let dev = R { x: theme.x - GROUP_GAP - BTN, ..theme };
        let pip = R { x: dev.x - 2.0 - BTN, ..theme };
        let hist = R { x: pip.x - GROUP_GAP - BTN, ..theme };
        let bookmarks = R { x: hist.x - 2.0 - BTN, ..theme };
        let star = R { x: bookmarks.x - 2.0 - BTN, ..theme };
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
            let i = self.index_at(self.editor_x(x));
            self.ed.move_to(i, true);
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
        self.end_tip();
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

    /// A window x position as a distance from the start of the address bar's text, in logical px.
    fn editor_x(&self, x: f32) -> f32 {
        let o = self.geo().omni;
        x - (o.x + 34.0) + self.scroll_x
    }

    /// Width in logical px of the first `n` characters of the address bar text.
    fn width_of_chars(&mut self, n: usize) -> f32 {
        let prefix: String = self.buf.chars().take(n).collect();
        let s = self.s;
        match &mut self.engine {
            Some(e) => e.text_width(&prefix, URL_FONT, Weight::Regular, s),
            None => prefix.chars().count() as f32 * URL_FONT * 0.55,
        }
    }

    /// The character boundary nearest to `lx` logical px from the start of the text.
    fn index_at(&mut self, lx: f32) -> usize {
        let n = self.buf.chars().count();
        if lx <= 0.0 {
            return 0;
        }
        // Widths only grow with the prefix, so bisect for the first boundary at or past `lx`.
        let (mut lo, mut hi) = (0usize, n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.width_of_chars(mid) < lx {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo > 0 && lo <= n {
            let (before, at) = (self.width_of_chars(lo - 1), self.width_of_chars(lo));
            if lx - before < at - lx {
                return lo - 1;
            }
        }
        lo.min(n)
    }

    /// The word around character `i` (or the run of non-word characters it sits in).
    fn word_at(&self, i: usize) -> (usize, usize) {
        let chars: Vec<char> = self.buf.chars().collect();
        if chars.is_empty() {
            return (0, 0);
        }
        let i = i.min(chars.len() - 1);
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let kind = word(chars[i]);
        let (mut lo, mut hi) = (i, i + 1);
        while lo > 0 && word(chars[lo - 1]) == kind {
            lo -= 1;
        }
        while hi < chars.len() && word(chars[hi]) == kind {
            hi += 1;
        }
        (lo, hi)
    }

    pub fn mouse_down(&mut self, button: MouseButton, x: f32, y: f32) {
        self.mouse = (x, y);
        let hit = self.hit(x, y);
        self.cmds.push(ChromeCmd::Dismiss);
        self.end_hover_card();
        self.end_tip();
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
                    let i = self.index_at(self.editor_x(x));
                    match n {
                        1 => self.ed.move_to(i, false),
                        2 => {
                            let (lo, hi) = self.word_at(i);
                            self.ed = Editor { anchor: lo, caret: hi };
                        }
                        _ => self.ed.select_all(self.buf.chars().count()),
                    }
                    self.selecting = n == 1;
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

    // ---------- button tooltips ----------

    /// Tooltip text for a toolbar or window button, with its shortcut.
    fn tip_text(&self, h: Hit) -> Option<&'static str> {
        Some(match h {
            Hit::Back => "Back (Alt+Left)",
            Hit::Forward => "Forward (Alt+Right)",
            Hit::Reload if self.model.loading => "Stop loading",
            Hit::Reload => "Reload (Ctrl+R)",
            Hit::Star if self.model.url.is_empty() => return None,
            Hit::Star if self.model.bookmarked => "Remove bookmark (Ctrl+D)",
            Hit::Star => "Bookmark this page (Ctrl+D)",
            Hit::Bookmarks => "Bookmarks (Ctrl+B)",
            Hit::Pip => "Picture in picture (Alt+P)",
            Hit::History => "Search history (Ctrl+H)",
            Hit::Theme if self.model.update_ready => "Update ready: restart from this menu",
            Hit::Theme => "Persona and theme",
            Hit::Devtools => "Developer tools (F12)",
            Hit::NewTab => "New tab (Ctrl+T)",
            Hit::Min => "Minimize",
            Hit::Max if self.model.maximized => "Restore down",
            Hit::Max => "Maximize",
            Hit::Close => "Close window",
            _ => return None,
        })
    }

    fn tip_rect(&self, h: Hit) -> Option<R> {
        let g = self.geo();
        Some(match h {
            Hit::Back => g.back,
            Hit::Forward => g.fwd,
            Hit::Reload => g.reload,
            Hit::Star => g.star,
            Hit::Bookmarks => g.bookmarks,
            Hit::Pip => g.pip,
            Hit::History => g.hist,
            Hit::Theme => g.theme,
            Hit::Devtools => g.dev,
            Hit::NewTab => g.new_tab,
            Hit::Min => g.min,
            Hit::Max => g.max,
            Hit::Close => g.close,
            _ => return None,
        })
    }

    fn end_tip(&mut self) {
        if self.tip.take().is_some_and(|(_, _, shown)| shown) {
            self.cmds.push(ChromeCmd::TipEnd);
        }
    }

    fn update_tip(&mut self, now: Instant) {
        let busy = self.drag.is_some() || self.selecting || self.pressed.is_some();
        let over = if busy || self.tip_text(self.hover).is_none() { None } else { Some(self.hover) };
        match (over, self.tip) {
            (None, _) => self.end_tip(),
            (Some(h), Some((cur, _, _))) if h == cur => {}
            (Some(h), prev) => {
                // Moving between neighbouring buttons while a tip is up swaps it at once.
                let was_shown = prev.is_some_and(|(_, _, shown)| shown);
                self.tip = Some((h, if was_shown { now - TIP_DELAY } else { now }, false));
            }
        }
        if let Some((h, since, false)) = self.tip
            && now >= since + TIP_DELAY
            && let (Some(text), Some(r)) = (self.tip_text(h), self.tip_rect(h))
        {
            // The tab hover card and a tooltip never show together.
            self.cmds.push(ChromeCmd::Tip { text: text.to_owned(), x: (r.x + r.w / 2.0) as f64, y: (r.y + r.h + 6.0) as f64 });
            self.tip = Some((h, since, true));
        }
    }

    // ---------- address bar text ----------

    fn text(&self) -> String {
        self.buf.clone()
    }

    fn set_text(&mut self, text: &str, select_all: bool) {
        self.buf = text.chars().filter(|c| !c.is_control()).collect();
        let n = self.buf.chars().count();
        self.ed = if select_all { Editor { anchor: 0, caret: n } } else { Editor::at_end(n) };
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
        self.ed.insert(&mut self.buf, &clean, 2048, &InputKind::Text);
        self.query();
    }

    fn clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut()
    }

    fn select_all(&mut self) {
        self.ed.select_all(self.buf.chars().count());
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
            (Key::F11, _) => Some(ChromeCmd::ToggleFullscreen),
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
            // Ctrl+1..8 jump to that tab, Ctrl+9 to the last one.
            (_, Some(d)) if ctrl && !alt && d.len() == 1 && d.as_bytes()[0].is_ascii_digit() && d != "0" => {
                let n = (d.as_bytes()[0] - b'1') as usize;
                let ids = self.live_ids();
                let pick = if n == 8 { ids.last() } else { ids.get(n) };
                pick.map(|&id| ChromeCmd::Switch { id })
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
            Key::ArrowLeft => self.ed.left(&self.buf, shift, ctrl),
            Key::ArrowRight => self.ed.right(&self.buf, shift, ctrl),
            Key::Home => self.ed.home(shift),
            Key::End => self.ed.end(&self.buf, shift),
            Key::Backspace => {
                self.ed.backspace(&mut self.buf, ctrl);
                self.query();
            }
            Key::Delete => {
                self.ed.delete_forward(&mut self.buf, ctrl);
                self.query();
            }
            Key::Character(_) if ctrl => match ch.as_deref() {
                Some("a") => self.select_all(),
                Some("c") | Some("x") => {
                    let sel = self.ed.selected_text(&self.buf);
                    if !sel.is_empty() {
                        if let Some(cb) = self.clipboard() {
                            let _ = cb.set_text(sel);
                        }
                        if ch.as_deref() == Some("x") {
                            self.ed.delete_selection(&mut self.buf);
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
        self.update_tip(now);
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
            busy |= approach01(&mut self.hover_fade[hit_key(h)], target, kf);
        }

        // The hover gradient turns slowly while any button is lit (the address bar has no gradient).
        if self.hover_fade.iter().enumerate().any(|(i, f)| i != hit_key(Hit::Omni) && *f > 0.01) {
            self.shimmer = (self.shimmer + dt * 0.45).fract();
            busy = true;
        }
        busy |= approach01(&mut self.load_morph, if self.model.loading { 1.0 } else { 0.0 }, kf);
        busy |= approach01(&mut self.focus_f, if self.editing { 1.0 } else { 0.0 }, kf);
        if self.star_pop > 0.0 {
            self.star_pop = (self.star_pop - dt / 0.45).max(0.0);
            busy = true;
        }
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
        if let Some((_, since, false)) = self.tip {
            wake(since + TIP_DELAY);
        }
        if self.editing {
            // Caret blink.
            let phase = now.saturating_duration_since(self.blink_from).as_millis() % 530;
            wake(now + Duration::from_millis((530 - phase) as u64));
        }
        next
    }

    // ---------- drawing ----------

    /// Draws the chrome with the ANVL engine into `pm`, the strip `main.rs` presents.
    pub fn render(&mut self, pm: &mut Pixmap) {
        let th = self.theme;
        let Some(mut engine) = self.engine.take() else {
            // No GPU: at least paint the bar's color, so the window is not left undrawn.
            pm.fill(tiny_skia::Color::from_rgba8(th.toolbar.0, th.toolbar.1, th.toolbar.2, 255));
            return;
        };
        self.sync_textures(&mut engine);
        let (w, h, s) = (pm.width(), pm.height(), self.s);
        let frame = engine.frame(w, h, s, Rgba(th.toolbar.0, th.toolbar.1, th.toolbar.2, 255), |p| self.paint(p));
        anvil::copy_to_pixmap(&frame, pm);
        self.engine = Some(engine);
    }

    /// Makes sure every picture the chrome shows is on the GPU: favicons, the logo, the persona
    /// picture and the wallpaper strip.
    fn sync_textures(&mut self, engine: &mut Engine) {
        for (url, img) in &self.icons {
            if let Some(img) = img {
                engine.ensure_texture(url, img);
            }
        }
        if let Some(logo) = &self.logo {
            engine.ensure_texture("persona:logo", logo);
        }
        if !self.bg_uploaded {
            engine.forget_texture("persona:bg");
            if let Some(bg) = &self.bg {
                engine.ensure_texture("persona:bg", bg);
            }
            self.bg_uploaded = true;
        }
    }

    fn paint(&mut self, c: &mut Painter) {
        let s = self.s;
        let th = self.theme;
        c.backdrop = th.strip;
        c.rect(0.0, 0.0, self.width, TITLE_H, th.strip);
        c.rect(0.0, TITLE_H, self.width, HEIGHT - TITLE_H, th.toolbar);
        if self.bg.is_some() && c.image("persona:bg", 0.0, 0.0, self.width, HEIGHT, 0.44) {
            c.rect(0.0, 0.0, self.width, TITLE_H, th.strip.with_alpha(0.56));
            c.rect(0.0, TITLE_H, self.width, HEIGHT - TITLE_H, th.toolbar.with_alpha(0.74));
        }

        self.draw_tabs(c, &th);
        let g = self.geo();

        // New tab button.
        self.button_bg(c, g.new_tab, Hit::NewTab, 8.0, &th);
        // The plus turns a quarter as the pointer arrives.
        let k = self.press_scale(Hit::NewTab);
        let sz = 16.0 * k;
        c.icon_rot(Icon::Plus, g.new_tab.x + 7.0 + (16.0 - sz) / 2.0, g.new_tab.y + 7.0 + (16.0 - sz) / 2.0, sz, th.dim.mix(th.fg, self.fade(Hit::NewTab)), self.fade(Hit::NewTab) * std::f32::consts::FRAC_PI_2);

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
            c.icon(icon, r.x + (r.w - 16.0) / 2.0, r.y + (r.h - 16.0) / 2.0, 16.0, fg);
        }

        // Toolbar buttons (translucent icons blend against the toolbar color).
        c.backdrop = th.toolbar;
        let disabled = |on: bool| if on { 1.0 } else { 0.3 };
        for (r, h, icon, on) in [
            (g.back, Hit::Back, Icon::Back, self.model.can_back),
            (g.fwd, Hit::Forward, Icon::Forward, self.model.can_forward),
            (g.bookmarks, Hit::Bookmarks, Icon::Bookmarks, true),
            (g.hist, Hit::History, Icon::History, true),
        ] {
            if on {
                self.button_bg(c, r, h, 9.0, &th);
            }
            let sz = 20.0 * self.press_scale(h);
            c.icon(icon, r.x + 7.0 + (20.0 - sz) / 2.0, r.y + 7.0 + (20.0 - sz) / 2.0, sz, th.fg.with_alpha(disabled(on)));
        }
        // Star (bookmark current page) button.
        let can_star = !self.model.url.is_empty();
        if self.model.bookmarked {
            c.rrect(g.star.x, g.star.y, g.star.w, g.star.h, 9.0, th.accent.with_alpha(0.14));
        } else if can_star {
            self.button_bg(c, g.star, Hit::Star, 9.0, &th);
        }
        // A little pop when a page gets bookmarked.
        let pop = 1.0 + 0.3 * (self.star_pop * std::f32::consts::PI).sin();
        let ssz = 20.0 * pop * self.press_scale(Hit::Star);
        c.icon(
            if self.model.bookmarked { Icon::StarFilled } else { Icon::Star },
            g.star.x + 7.0 + (20.0 - ssz) / 2.0,
            g.star.y + 7.0 + (20.0 - ssz) / 2.0,
            ssz,
            if self.model.bookmarked { th.accent } else { th.fg.with_alpha(disabled(can_star)) },
        );
        self.button_bg(c, g.reload, Hit::Reload, 9.0, &th);
        let m = self.load_morph;
        // Reload spins away into Stop and back.
        let rs = 20.0 * self.press_scale(Hit::Reload);
        let (rx, ry) = (g.reload.x + 7.0 + (20.0 - rs) / 2.0, g.reload.y + 7.0 + (20.0 - rs) / 2.0);
        c.icon_rot(Icon::Reload, rx, ry, rs, th.fg.with_alpha(1.0 - m), m * std::f32::consts::PI);
        c.icon_rot(Icon::Stop, rx, ry, rs, th.fg.with_alpha(m), (m - 1.0) * std::f32::consts::PI);
        self.button_bg(c, g.theme, Hit::Theme, 9.0, &th);
        self.draw_avatar(c, g.theme, &th);
        for (r, h, icon, active) in [
            (g.pip, Hit::Pip, Icon::Pip, self.model.pip),
            (g.dev, Hit::Devtools, Icon::Code, self.model.devtools),
        ] {
            if active {
                c.rrect(r.x, r.y, r.w, r.h, 9.0, th.accent.with_alpha(0.14));
            } else {
                self.button_bg(c, r, h, 9.0, &th);
            }
            let sz = 20.0 * self.press_scale(h);
            c.icon(icon, r.x + 7.0 + (20.0 - sz) / 2.0, r.y + 7.0 + (20.0 - sz) / 2.0, sz, if active { th.accent } else { th.fg });
        }

        // Thin dividers between the button groups.
        for left in [g.pip, g.theme] {
            let x = left.x - GROUP_GAP / 2.0 + 1.0;
            c.hairline_v(x, ROW_Y + 8.0, BTN - 16.0, th.border);
        }

        self.draw_omni(c, g.omni, &th);

        // Page-load progress along the bottom edge, then the hairline under the toolbar.
        c.hairline_h(0.0, HEIGHT - 1.0 / s, self.width, th.border);
        if self.progress_alpha > 0.0 {
            let w = self.width * self.progress;
            c.rect(0.0, HEIGHT - 2.0, w, 2.0, th.accent.mix(th.accent2, 0.35).with_alpha(self.progress_alpha));
        }
    }

    /// Buttons dip a little while pressed.
    fn press_scale(&self, h: Hit) -> f32 {
        if self.pressed == Some(h) && self.hover == h { 0.86 } else { 1.0 }
    }

    fn fade(&self, h: Hit) -> f32 {
        self.hover_fade[hit_key(h)]
    }

    fn button_bg(&self, c: &mut Painter, r: R, h: Hit, radius: f32, th: &Theme) {
        let pressed = self.pressed == Some(h) && self.hover == h;
        if pressed {
            c.rrect(r.x, r.y, r.w, r.h, radius, th.btn_press);
            return;
        }
        let f = self.fade(h);
        if f > 0.01 {
            c.rrect(r.x, r.y, r.w, r.h, radius, th.btn_hover.with_alpha(f));
            // Faint Gemini-style glow, tinted by the persona's accent, turning slowly.
            let phase = (self.shimmer + hit_key(h) as f32 * 0.11).fract();
            c.ai_glow(r.x, r.y, r.w, r.h, radius, f, phase, th.accent, th.fg.0 > 128);
        }
    }

    fn draw_tabs(&mut self, c: &mut Painter, th: &Theme) {
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
                    c.rrect(x - spread, TAB_Y - spread * 0.3, w + spread * 2.0, TAB_H + spread, 10.0 + spread, Rgba(0, 0, 0, 255).with_alpha(a));
                }
                c.rrect(x, TAB_Y, w, TAB_H, 10.0, th.toolbar);
            } else if is_active {
                c.rrect_top(x, TAB_Y, w, TAB_H, 10.0, th.toolbar);
                c.foot(x - 10.0, TITLE_H, 10.0, false, th.toolbar);
                c.foot(x + w, TITLE_H, 10.0, true, th.toolbar);
            } else if hover > 0.0 {
                c.rrect_top(x, TAB_Y, w, TAB_H, 10.0, th.tab_hover.with_alpha(hover * alpha));
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
    fn draw_tab_content(&mut self, c: &mut Painter, i: usize, x: f32, w: f32, alpha: f32, active: bool, th: &Theme) {
        let id = self.vis[i].id;
        c.backdrop = if active { th.toolbar } else { th.strip };
        let Some(info) = self.model.tabs.iter().find(|t| t.id == id) else { return };
        let (title, favicon, loading, pip) = (info.title.clone(), info.favicon.clone(), info.loading, info.pip);
        let hovered = matches!(self.hover, Hit::Tab(h) | Hit::TabClose(h) if h == id);
        let narrow = w < NARROW_W;
        let fg = if active || hovered { th.fg } else { th.dim }.with_alpha(alpha);
        let cy = TAB_Y + TAB_H / 2.0;
        let icon_x = if narrow && !active { x + (w - 16.0) / 2.0 } else { x + 12.0 };
        let show_close = !self.vis[i].leaving && (active || hovered) && (!narrow || active) && w >= 44.0;
        let close = self.close_rect(&self.vis[i]);
        // Favicon, spinner, or the Persona logo badge.
        let has_favicon = matches!(self.icons.get(&favicon), Some(Some(_)));
        if loading {
            c.spinner(icon_x + 8.0, cy, 6.0, self.spin, th.accent.with_alpha(alpha));
        } else if (favicon == BRAND_ICON || (favicon.is_empty() && (title.is_empty() || title == "New Tab")))
            && self.logo.is_some()
            && c.image("persona:logo", icon_x, cy - 8.0, 16.0, 16.0, alpha)
        {
        } else if favicon == BRAND_ICON {
            c.rrect(icon_x, cy - 8.0, 16.0, 16.0, 4.0, th.accent.mix(th.accent2, 0.3).with_alpha(alpha));
            let letter = if !self.model.badge_letter.is_empty() {
                self.model.badge_letter.as_str()
            } else {
                self.model.brand.badge_letter()
            };
            let lw = c.text_width(letter, 11.0, Weight::SemiBold);
            let lh = c.line_height(11.0, Weight::SemiBold);
            c.text(letter, icon_x + (16.0 - lw) / 2.0, cy - lh / 2.0, 11.0, Weight::SemiBold, Rgba(255, 255, 255, 255).with_alpha(alpha), f32::MAX);
        } else if has_favicon && c.image(&favicon, icon_x, cy - 8.0, 16.0, 16.0, alpha) {
        } else {
            c.icon(Icon::Globe, icon_x, cy - 8.0, 16.0, th.faint.with_alpha(alpha));
        }
        if pip && !loading {
            let ring = if active { th.toolbar } else { th.strip };
            c.circle(icon_x + 14.5, cy + 6.5, 4.5, ring.with_alpha(alpha));
            c.circle(icon_x + 14.5, cy + 6.5, 3.0, th.accent.with_alpha(alpha));
        }
        if !narrow {
            let text = if title.is_empty() { "New Tab".to_owned() } else { title };
            let clip = if show_close { close.x - 2.0 } else { x + w - 7.0 };
            let lh = c.line_height(FONT, Weight::Regular);
            c.text(&text, x + 36.0, cy - lh / 2.0, FONT, Weight::Regular, fg, clip);
        }
        if show_close {
            if self.hover == Hit::TabClose(id) {
                c.circle(close.x + 11.0, close.y + 11.0, 11.0, th.btn_hover);
            }
            c.icon(Icon::Close, close.x + 5.5, close.y + 5.5, 11.0, fg.with_alpha(0.75 * alpha));
        }
    }

    /// The persona button: the active persona's profile picture in a circle, or its initial on
    /// its color when it has none.
    fn draw_avatar(&mut self, c: &mut Painter, r: R, th: &Theme) {
        const D: f32 = 24.0;
        let (x, y) = (r.x + (r.w - D) / 2.0, r.y + (r.h - D) / 2.0);
        let has_pfp = !self.model.pfp.is_empty() && matches!(self.icons.get(&self.model.pfp), Some(Some(_)));
        if !(has_pfp && c.image_cover(&self.model.pfp, x, y, D, D, D / 2.0)) {
            c.circle(x + D / 2.0, y + D / 2.0, D / 2.0, th.accent);
            let letter = if self.model.badge_letter.is_empty() { self.model.brand.badge_letter().to_owned() } else { self.model.badge_letter.clone() };
            let (lw, lh) = (c.text_width(&letter, 12.0, Weight::SemiBold), c.line_height(12.0, Weight::SemiBold));
            c.text(&letter, x + (D - lw) / 2.0, y + (D - lh) / 2.0, 12.0, Weight::SemiBold, Rgba(255, 255, 255, 255), f32::MAX);
        }
        if self.model.update_ready {
            // Update badge: a dot on the top-right of the avatar, ringed in the toolbar color.
            let (cx, cy) = (r.x + r.w / 2.0 + 9.0, r.y + r.h / 2.0 - 9.0);
            c.circle(cx, cy, 5.0, th.toolbar);
            c.circle(cx, cy, 3.5, Rgba(52, 199, 89, 255));
        }
    }

    fn draw_omni(&mut self, c: &mut Painter, o: R, th: &Theme) {
        let f = if self.editing { 0.0 } else { self.fade(Hit::Omni) };
        // The focus ring fades and grows in rather than appearing at once.
        let ff = self.focus_f;
        if ff > 0.01 {
            let grow = 3.0 * ff;
            c.rrect(o.x - grow, o.y - grow, o.w + grow * 2.0, o.h + grow * 2.0, 17.0 + grow, th.accent.with_alpha(0.22 * ff));
            c.rrect(o.x - 1.0, o.y - 1.0, o.w + 2.0, o.h + 2.0, 18.0, th.accent.with_alpha(ff));
        }
        c.rrect(o.x, o.y, o.w, o.h, 17.0, th.field.mix(th.field_hover, f));

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
        let ty = o.y + (o.h - c.line_height(URL_FONT, Weight::Regular)) / 2.0;
        if self.editing {
            self.draw_editor(c, tx, ty, right, th);
        } else if url.is_empty() {
            c.text("Search or type a URL", tx, ty, URL_FONT, Weight::Regular, th.faint, right);
        } else {
            Self::draw_pretty_url(c, &url, tx, ty, right, th);
        }
    }

    /// The URL with the scheme (for https) and "www." hidden, and the host emphasised.
    fn draw_pretty_url(c: &mut Painter, url: &str, x: f32, y: f32, right: f32, th: &Theme) {
        let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
        let rest = rest.strip_prefix("www.").unwrap_or(rest);
        let split = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (host, path) = rest.split_at(split);
        let path = if path == "/" { "" } else { path };
        let prefix = if scheme == "http" { "http://" } else { "" };
        let mut x = x;
        for (part, col) in [(prefix, th.dim), (host, th.fg), (path, th.dim)] {
            if x >= right {
                break;
            }
            x += c.text(part, x, y, URL_FONT, Weight::Regular, col, right);
        }
    }

    fn draw_editor(&mut self, c: &mut Painter, x: f32, y: f32, right: f32, th: &Theme) {
        let lh = c.line_height(URL_FONT, Weight::Regular);
        let width_to = |c: &mut Painter, text: &str, n: usize| {
            let prefix: String = text.chars().take(n).collect();
            c.text_width(&prefix, URL_FONT, Weight::Regular)
        };
        if self.buf.is_empty() {
            c.text("Search or type a URL", x, y, URL_FONT, Weight::Regular, th.faint, right);
        }
        // Keep the caret in view by scrolling the text horizontally.
        let avail = right - x;
        let cx = width_to(c, &self.buf, self.ed.caret);
        if cx - self.scroll_x > avail - 2.0 {
            self.scroll_x = cx - avail + 2.0;
        } else if cx < self.scroll_x {
            self.scroll_x = cx;
        }
        let ox = x - self.scroll_x;
        if self.ed.has_selection() {
            let (lo, hi) = self.ed.selection();
            let (a, b) = (ox + width_to(c, &self.buf, lo), ox + width_to(c, &self.buf, hi));
            let (a, b) = (a.max(x), b.min(right));
            if b > a {
                c.rect(a, y, b - a, lh, th.selection);
            }
        }
        c.text_cut(&self.buf, ox, y, URL_FONT, Weight::Regular, th.fg, (x, right));
        if self.blink_from.elapsed().as_millis() / 530 % 2 == 0 {
            let cx = ox + cx;
            if cx >= x - 0.5 && cx <= right {
                c.rect(cx, y, 1.5, lh, th.fg);
            }
        }
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
        for (name, dark, editing) in [("toolbar-dark", true, false), ("toolbar-light", false, false), ("toolbar-editing", true, true), ("toolbar-pfp", true, false)] {
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
                badge_letter: "M".into(),
                pfp: if name == "toolbar-pfp" { "data:test-pfp".into() } else { String::new() },
                update_ready: name == "toolbar-pfp",
                ..Model::default()
            });
            if name == "toolbar-pfp" {
                // A stand-in picture: a diagonal two-color split, to show the circular crop.
                let mut img = Pixmap::new(64, 48).unwrap();
                img.fill(tiny_skia::Color::from_rgba8(230, 120, 60, 255));
                let tri = tiny_skia::PathBuilder::from_rect(tiny_skia::Rect::from_xywh(0.0, 24.0, 64.0, 24.0).unwrap());
                let mut paint = tiny_skia::Paint::default();
                paint.set_color_rgba8(60, 110, 220, 255);
                img.fill_path(&tri, &paint, tiny_skia::FillRule::Winding, tiny_skia::Transform::identity(), None);
                tb.icon_loaded("data:test-pfp".into(), Some(img));
            }
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

