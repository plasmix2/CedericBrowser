//! The New Tab page, drawn natively by the ANVL engine into the browser window's own surface
//! (the page area under the toolbar). A New Tab has no WebView until it navigates somewhere.
//!
//! Message-in, state-out: `Browser` feeds it data (`set_data`, `set_theme`, pictures) and pointer
//! events, and acts on the `HomeCmd` they return. Hit areas are recorded while painting, so they
//! always match what is on screen.

use std::collections::HashMap;

use tao::window::CursorIcon;
use tiny_skia::Pixmap;

use crate::{
    anvil::{self, Engine, Painter, Weight},
    gfx::{Icon, PersonaColor, Rgba, Theme},
};

/// What a click on the New Tab page asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum HomeCmd {
    /// The search box: put the keyboard in the address bar.
    FocusSearch,
    Open(String),
    DeleteBookmark(String),
    Settings,
    ChooseBackground,
    ClearBackground,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tile {
    pub title: String,
    pub url: String,
}

/// What the page shows; set by `Browser` whenever it changes.
#[derive(Clone, Debug, Default)]
pub struct HomeData {
    #[allow(dead_code)]
    pub persona: String,
    #[allow(dead_code)]
    pub letter: String,
    pub has_bg: bool,
    pub tiles: Vec<Tile>,
    /// The persona's picture (a URL in the shared image cache), if any.
    pub avatar: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Search,
    Settings,
    Background,
    ClearBg,
    Tile(usize),
    TileDel(usize),
}

#[derive(Clone, Copy)]
struct Area {
    hit: Hit,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

pub struct Home {
    engine: Option<Engine>,
    theme: Theme,
    color: PersonaColor,
    dark: bool,
    s: f32,
    w: f32,
    h: f32,
    data: HomeData,
    bg: Option<Pixmap>,
    bg_uploaded: bool,
    logo: Option<Pixmap>,
    images: HashMap<String, Pixmap>,
    uploaded: Vec<String>,
    areas: Vec<Area>,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    dirty: bool,
    /// The last rendered page, `w * s` by `h * s` pixels.
    pub frame: Pixmap,
}

const PILL_H: f32 = 34.0;
const TITLE: &str = "Persona Browser";
const SEARCH_W: f32 = 580.0;
const SEARCH_H: f32 = 54.0;
const CHIP_H: f32 = 36.0;
/// How far outside its box a click still counts (a slight wobble while clicking).
const SLOP: f32 = 4.0;

impl Home {
    pub fn new(engine: Option<Engine>, dark: bool, color: PersonaColor) -> Self {
        Self {
            engine,
            theme: Theme::for_persona(dark, color),
            color,
            dark,
            s: 1.0,
            w: 1.0,
            h: 1.0,
            data: HomeData::default(),
            bg: None,
            bg_uploaded: false,
            logo: Pixmap::decode_png(include_bytes!("ui/logo.png")).ok(),
            images: HashMap::new(),
            uploaded: Vec::new(),
            areas: Vec::new(),
            hover: None,
            pressed: None,
            dirty: true,
            frame: Pixmap::new(1, 1).expect("1x1 pixmap"),
        }
    }

    pub fn set_theme(&mut self, dark: bool, color: PersonaColor) {
        if (dark, color) != (self.dark, self.color) {
            self.dark = dark;
            self.color = color;
            self.theme = Theme::for_persona(dark, color);
            self.dirty = true;
        }
    }

    /// The page color under everything.
    pub fn base(&self) -> Rgba {
        page_color(self.dark, self.color)
    }

    pub fn set_data(&mut self, data: HomeData) {
        self.data = data;
        self.hover = None;
        self.pressed = None;
        self.dirty = true;
    }

    #[allow(dead_code)]
    pub fn set_tiles(&mut self, tiles: Vec<Tile>) {
        if tiles != self.data.tiles {
            self.data.tiles = tiles;
            self.hover = None;
            self.pressed = None;
            self.dirty = true;
        }
    }

    pub fn set_background(&mut self, bg: Option<Pixmap>) {
        self.bg = bg;
        self.bg_uploaded = false;
        self.dirty = true;
    }

    pub fn image_loaded(&mut self, url: &str, img: Option<&Pixmap>) {
        if self.data.avatar.as_deref() == Some(url)
            && let Some(img) = img
        {
            self.images.insert(url.to_owned(), img.clone());
            self.uploaded.retain(|u| u != url);
            self.dirty = true;
        }
    }

    pub fn wants_image(&self) -> Option<&str> {
        self.data.avatar.as_deref().filter(|u| !self.images.contains_key(*u))
    }

    pub fn set_size(&mut self, scale: f32, w: f32, h: f32) {
        if (scale, w, h) != (self.s, self.w, self.h) {
            (self.s, self.w, self.h) = (scale, w.max(1.0), h.max(1.0));
            self.dirty = true;
        }
    }

    /// Frees the page's picture (a full window of pixels, here and on the GPU) while no New Tab
    /// is on screen; it is drawn again when one is.
    pub fn release(&mut self) {
        if self.frame.width() > 1 {
            self.frame = Pixmap::new(1, 1).expect("1x1 pixmap");
            self.dirty = true;
            if let Some(e) = &self.engine {
                e.trim(crate::popups::TRIM_PIXELS);
            }
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Re-draws the page into `frame` if anything changed. Returns true when it did.
    pub fn render(&mut self) -> bool {
        let (pw, ph) = ((self.w * self.s).round().max(1.0) as u32, (self.h * self.s).round().max(1.0) as u32);
        if !self.dirty && self.frame.width() == pw && self.frame.height() == ph {
            return false;
        }
        self.dirty = false;
        if self.frame.width() != pw || self.frame.height() != ph {
            let Some(pm) = Pixmap::new(pw, ph) else { return false };
            self.frame = pm;
        }
        let base = self.base();
        let Some(mut engine) = self.engine.take() else {
            self.frame.fill(tiny_skia::Color::from_rgba8(base.0, base.1, base.2, 255));
            return true;
        };
        if !self.bg_uploaded {
            engine.forget_texture("home:bg");
            if let Some(bg) = &self.bg {
                engine.ensure_texture("home:bg", bg);
            }
            self.bg_uploaded = true;
        }
        if let Some(logo) = &self.logo {
            engine.ensure_texture("home:logo", logo);
        }
        for (url, img) in &self.images {
            if !self.uploaded.contains(url) {
                engine.forget_texture(url);
                engine.ensure_texture(url, img);
                self.uploaded.push(url.clone());
            }
        }
        let frame = engine.frame(pw, ph, self.s, base, |p| self.paint(p));
        anvil::copy_to_pixmap(&frame, &mut self.frame);
        self.engine = Some(engine);
        true
    }

    fn area(&mut self, hit: Hit, x: f32, y: f32, w: f32, h: f32) {
        self.areas.push(Area { hit, x, y, w, h });
    }

    fn hot(&self, h: Hit) -> f32 {
        if self.hover == Some(h) { 1.0 } else { 0.0 }
    }

    fn paint(&mut self, c: &mut Painter) {
        self.areas.clear();
        let th = self.theme;
        let (w, h) = (self.w, self.h);
        let photo = self.bg.is_some() && c.image_cover("home:bg", 0.0, 0.0, w, h, 0.0);
        if photo {
            // Keep text readable over any wallpaper picture.
            c.rect(0.0, 0.0, w, h, Rgba(0, 0, 0, 255).with_alpha(if self.dark { 0.35 } else { 0.16 }));
        }
        let on_photo = |c: Rgba| if photo { Rgba(255, 255, 255, 255).with_alpha(c.3 as f32 / 255.0) } else { c };
        let fg = on_photo(th.fg);
        let dim = if photo { Rgba(255, 255, 255, 210) } else { th.dim };
        let card = if photo { th.card.with_alpha(0.85) } else { self.base().mix(th.fg, if self.dark { 0.07 } else { 0.05 }) };
        c.backdrop = card;

        // Top right navigation buttons: Settings, Wallpaper, Remove Wallpaper
        let mut rx = w - 24.0;
        let ry = 18.0;
        let dark = self.dark;
        let pill_bg = if photo {
            if dark { Rgba(20, 24, 33, 215) } else { Rgba(255, 255, 255, 225) }
        } else {
            card
        };
        let pill_border = if photo {
            if dark { Rgba(255, 255, 255, 55) } else { Rgba(0, 0, 0, 40) }
        } else {
            th.border
        };
        let pill = |c: &mut Painter, px: f32, pw: f32, f: f32| {
            if photo {
                c.shadow(px, ry, pw, PILL_H, PILL_H / 2.0, 16.0, 4.0, if dark { 0.45 } else { 0.18 });
            }
            c.rrect(px - 1.0, ry - 1.0, pw + 2.0, PILL_H + 2.0, PILL_H / 2.0 + 1.0, pill_border.mix(th.accent, f));
            c.rrect(px, ry, pw, PILL_H, PILL_H / 2.0, pill_bg.mix(th.fg, 0.08 * f));
        };

        // 1. Settings button
        rx -= PILL_H;
        pill(c, rx, PILL_H, self.hot(Hit::Settings));
        c.icon(Icon::Settings, rx + (PILL_H - 16.0) / 2.0, ry + (PILL_H - 16.0) / 2.0, 16.0, fg);
        self.area(Hit::Settings, rx, ry, PILL_H, PILL_H);

        // 2. Wallpaper button
        let bg_label = if self.data.has_bg { "Change wallpaper" } else { "Wallpaper" };
        let lw = c.text_width(bg_label, 13.0, Weight::Regular);
        let bw = 12.0 + 16.0 + 8.0 + lw + 14.0;
        rx -= 10.0 + bw;
        pill(c, rx, bw, self.hot(Hit::Background));
        c.icon(Icon::Photo, rx + 12.0, ry + (PILL_H - 16.0) / 2.0, 16.0, fg);
        let ly = ry + (PILL_H - c.line_height(13.0, Weight::Regular)) / 2.0;
        c.text(bg_label, rx + 36.0, ly, 13.0, Weight::Regular, fg, 1.0e9);
        self.area(Hit::Background, rx, ry, bw, PILL_H);

        // 3. Clear wallpaper button (if wallpaper is active)
        if self.data.has_bg {
            rx -= 10.0 + PILL_H;
            pill(c, rx, PILL_H, self.hot(Hit::ClearBg));
            c.icon(Icon::Close, rx + (PILL_H - 16.0) / 2.0, ry + (PILL_H - 16.0) / 2.0, 16.0, fg);
            self.area(Hit::ClearBg, rx, ry, PILL_H, PILL_H);
        }

        // The centre column: logo, name, search box, bookmarks.
        let rows = self.tile_rows(c, w);
        let tiles_h = if rows.is_empty() { 0.0 } else { rows.len() as f32 * (CHIP_H + 10.0) + 18.0 };
        let block = 72.0 + 18.0 + 44.0 + 30.0 + SEARCH_H + tiles_h;
        let mut cy = ((h - block) / 2.0 - 15.0).max(60.0);
        let cx = w / 2.0;
        c.image_cover("home:logo", cx - 36.0, cy, 72.0, 72.0, 18.0);
        cy += 72.0 + 18.0;
        let tw = c.text_width(TITLE, 34.0, Weight::SemiBold);
        c.text(TITLE, cx - tw / 2.0, cy, 34.0, Weight::SemiBold, fg, 1.0e9);
        cy += 44.0 + 30.0;

        // Search box - beautifully contrast-tuned for custom wallpapers and solid themes.
        let sw = SEARCH_W.min(w - 40.0);
        let sx = cx - sw / 2.0;
        let f = self.hot(Hit::Search);

        let (field_bg, field_border, search_icon_col, placeholder_col) = if photo {
            let bg = if self.dark {
                Rgba(18, 22, 30, 240)
            } else {
                Rgba(255, 255, 255, 246)
            };
            let bdr = if self.dark {
                Rgba(255, 255, 255, 65).mix(th.accent, f)
            } else {
                Rgba(0, 0, 0, 48).mix(th.accent, f)
            };
            let icon_c = if self.dark {
                Rgba(255, 255, 255, 225).mix(th.accent, f)
            } else {
                Rgba(30, 35, 45, 225).mix(th.accent, f)
            };
            let ph_c = if self.dark {
                Rgba(255, 255, 255, 190)
            } else {
                Rgba(50, 55, 68, 190)
            };
            (bg, bdr, icon_c, ph_c)
        } else {
            let bg = if self.dark { th.field } else { card };
            let bdr = th.border.mix(th.accent, f);
            let icon_c = th.dim.mix(th.accent, f);
            let ph_c = th.dim;
            (bg, bdr, icon_c, ph_c)
        };

        c.shadow(
            sx,
            cy,
            sw,
            SEARCH_H,
            SEARCH_H / 2.0,
            if photo { 36.0 } else { 28.0 },
            if photo { 10.0 } else { 6.0 },
            if photo { if self.dark { 0.65 } else { 0.28 } } else { if self.dark { 0.32 } else { 0.10 } },
        );
        c.rrect(sx - 1.0, cy - 1.0, sw + 2.0, SEARCH_H + 2.0, SEARCH_H / 2.0 + 1.0, field_border);
        c.rrect(sx, cy, sw, SEARCH_H, SEARCH_H / 2.0, field_bg);
        c.backdrop = field_bg;
        c.icon(Icon::Search, sx + 20.0, cy + (SEARCH_H - 18.0) / 2.0, 18.0, search_icon_col);
        let sty = cy + (SEARCH_H - c.line_height(15.0, Weight::Regular)) / 2.0;
        c.text("Search the web or type a URL", sx + 50.0, sty, 15.0, Weight::Regular, placeholder_col, sx + sw - 20.0);
        self.area(Hit::Search, sx, cy, sw, SEARCH_H);
        cy += SEARCH_H + 28.0;

        // Bookmarks, as chips centred in rows.
        let (chip_card, chip_bdr_base) = if photo {
            let c_bg = if self.dark { Rgba(20, 24, 33, 225) } else { Rgba(255, 255, 255, 235) };
            let c_bdr = if self.dark { Rgba(255, 255, 255, 55) } else { Rgba(0, 0, 0, 38) };
            (c_bg, c_bdr)
        } else {
            (card, th.border)
        };
        for row in rows {
            let total: f32 = row.iter().map(|(_, cw)| cw).sum::<f32>() + 10.0 * (row.len().saturating_sub(1)) as f32;
            let mut x = cx - total / 2.0;
            for (i, cw) in row {
                let hot = self.hot(Hit::Tile(i)).max(self.hot(Hit::TileDel(i)));
                if photo {
                    c.shadow(x, cy, cw, CHIP_H, CHIP_H / 2.0, 16.0, 4.0, if self.dark { 0.45 } else { 0.18 });
                }
                c.rrect(x - 1.0, cy - 1.0, cw + 2.0, CHIP_H + 2.0, CHIP_H / 2.0 + 1.0, chip_bdr_base.mix(th.accent, hot));
                c.rrect(x, cy, cw, CHIP_H, CHIP_H / 2.0, chip_card.mix(th.fg, 0.06 * hot));
                c.icon(Icon::StarFilled, x + 12.0, cy + (CHIP_H - 13.0) / 2.0, 13.0, th.accent);
                let t = &self.data.tiles[i];
                let label = if t.title.is_empty() { t.url.clone() } else { t.title.clone() };
                let lty = cy + (CHIP_H - c.line_height(13.0, Weight::Regular)) / 2.0;
                c.text(&label, x + 32.0, lty, 13.0, Weight::Regular, fg, x + cw - 30.0);
                self.area(Hit::Tile(i), x, cy, cw, CHIP_H);
                // The remove button is always there (and clickable), shown faintly until hovered.
                let (dx, dy) = (x + cw - 28.0, cy + (CHIP_H - 22.0) / 2.0);
                if self.hover == Some(Hit::TileDel(i)) {
                    c.circle(dx + 11.0, dy + 11.0, 11.0, th.btn_hover);
                }
                c.icon(Icon::Close, dx + 4.0, dy + 4.0, 14.0, dim.with_alpha(if hot > 0.0 { 1.0 } else { 0.55 }));
                self.area(Hit::TileDel(i), dx, dy, 22.0, 22.0);
                x += cw + 10.0;
            }
            cy += CHIP_H + 10.0;
        }
    }

    /// Bookmark chips laid out in centred rows: (index, width) per chip.
    fn tile_rows(&self, c: &mut Painter, w: f32) -> Vec<Vec<(usize, f32)>> {
        let max_row = (SEARCH_W + 100.0).min(w - 40.0);
        let mut rows: Vec<Vec<(usize, f32)>> = Vec::new();
        let mut used = 0.0;
        for (i, t) in self.data.tiles.iter().enumerate().take(24) {
            let label = if t.title.is_empty() { &t.url } else { &t.title };
            let cw = (32.0 + c.text_width(label, 13.0, Weight::Regular) + 36.0).min(220.0);
            if rows.is_empty() || used + cw + 10.0 > max_row {
                if rows.len() == 3 {
                    break;
                }
                rows.push(Vec::new());
                used = 0.0;
            }
            used += cw + 10.0;
            if let Some(r) = rows.last_mut() {
                r.push((i, cw));
            }
        }
        rows
    }

    fn hit_at(&self, x: f32, y: f32, slop: f32) -> Option<Hit> {
        // Later areas are drawn on top (the remove button inside a chip).
        self.areas
            .iter()
            .rev()
            .find(|a| x >= a.x - slop && x < a.x + a.w + slop && y >= a.y - slop && y < a.y + a.h + slop)
            .map(|a| a.hit)
    }

    /// Pointer moved to (x, y) in page coordinates. Returns the cursor to show.
    pub fn mouse_move(&mut self, x: f32, y: f32) -> CursorIcon {
        let hit = self.hit_at(x, y, 0.0);
        if hit != self.hover {
            self.hover = hit;
            self.dirty = true;
        }
        match hit {
            Some(Hit::Search) => CursorIcon::Text,
            Some(_) => CursorIcon::Hand,
            None => CursorIcon::Default,
        }
    }

    pub fn mouse_left(&mut self) {
        if self.hover.take().is_some() {
            self.dirty = true;
        }
        self.pressed = None;
    }

    pub fn mouse_down(&mut self, x: f32, y: f32) -> Option<HomeCmd> {
        self.mouse_move(x, y);
        self.pressed = self.hit_at(x, y, 0.0);
        // The search box acts on press, like the address bar it stands for.
        (self.pressed == Some(Hit::Search)).then_some(HomeCmd::FocusSearch)
    }

    pub fn mouse_up(&mut self, x: f32, y: f32) -> Option<HomeCmd> {
        let pressed = self.pressed.take()?;
        // Released on the pressed item, or just beside it.
        let on = self.hit_at(x, y, 0.0) == Some(pressed)
            || self.areas.iter().any(|a| a.hit == pressed && x >= a.x - SLOP && x < a.x + a.w + SLOP && y >= a.y - SLOP && y < a.y + a.h + SLOP);
        if !on {
            return None;
        }
        let url = |i: usize| self.data.tiles.get(i).map(|t| t.url.clone());
        match pressed {
            Hit::Search => None,
            Hit::Settings => Some(HomeCmd::Settings),
            Hit::Background => Some(HomeCmd::ChooseBackground),
            Hit::ClearBg => Some(HomeCmd::ClearBackground),
            Hit::Tile(i) => url(i).map(HomeCmd::Open),
            Hit::TileDel(i) => url(i).map(HomeCmd::DeleteBookmark),
        }
    }
}

/// The New Tab page's base color for a persona (the tab strip's tint in dark mode, the toolbar's
/// in light mode, so the page continues the chrome).
pub fn page_color(dark: bool, color: PersonaColor) -> Rgba {
    let (strip, toolbar, _) = color.surface_tints(dark);
    if dark { strip } else { toolbar }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_map_to_commands_through_painted_areas() {
        let mut h = Home::new(None, true, PersonaColor::Orange);
        h.areas = vec![
            Area { hit: Hit::Tile(0), x: 10.0, y: 10.0, w: 100.0, h: 36.0 },
            Area { hit: Hit::TileDel(0), x: 84.0, y: 18.0, w: 20.0, h: 20.0 },
            Area { hit: Hit::Search, x: 0.0, y: 100.0, w: 200.0, h: 54.0 },
        ];
        h.set_tiles(vec![Tile { title: "Example".into(), url: "https://example.com/".into() }]);
        assert_eq!(h.mouse_down(20.0, 20.0), None);
        assert_eq!(h.mouse_up(20.0, 20.0), Some(HomeCmd::Open("https://example.com/".into())));
        h.mouse_down(90.0, 25.0);
        assert_eq!(h.mouse_up(90.0, 25.0), Some(HomeCmd::DeleteBookmark("https://example.com/".into())));
        // A release just outside the chip still counts; far away does not.
        h.mouse_down(20.0, 20.0);
        assert_eq!(h.mouse_up(12.0, 48.0), Some(HomeCmd::Open("https://example.com/".into())));
        h.mouse_down(20.0, 20.0);
        assert_eq!(h.mouse_up(300.0, 300.0), None);
        assert_eq!(h.mouse_down(50.0, 120.0), Some(HomeCmd::FocusSearch));
        assert_eq!(h.mouse_move(50.0, 120.0), CursorIcon::Text);

        // Top right buttons: Settings, Background, ClearBg
        h.areas.push(Area { hit: Hit::Settings, x: 500.0, y: 18.0, w: 34.0, h: 34.0 });
        h.areas.push(Area { hit: Hit::Background, x: 550.0, y: 18.0, w: 100.0, h: 34.0 });
        h.areas.push(Area { hit: Hit::ClearBg, x: 660.0, y: 18.0, w: 34.0, h: 34.0 });

        h.mouse_down(510.0, 25.0);
        assert_eq!(h.mouse_up(510.0, 25.0), Some(HomeCmd::Settings));

        h.mouse_down(570.0, 25.0);
        assert_eq!(h.mouse_up(570.0, 25.0), Some(HomeCmd::ChooseBackground));

        h.mouse_down(670.0, 25.0);
        assert_eq!(h.mouse_up(670.0, 25.0), Some(HomeCmd::ClearBackground));
    }
}
