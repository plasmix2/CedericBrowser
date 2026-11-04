#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod crypto;
mod devtools;
mod favicons;
mod gfx;
mod history;
mod input;
mod pacer;
mod popups;
mod suggest;
mod thumbs;
mod toolbar;

use std::{
    num::NonZeroU32,
    path::PathBuf,
    rc::Rc,
    sync::mpsc::Sender,
    time::Instant,
};

use gfx::Brand;
use history::HistoryStore;
use popups::{Card, Content, Kind as PopupKind, MenuEntry, Pick, Popups, SuggestEntry};
use tiny_skia::Pixmap;
use toolbar::{BRAND_ICON, Toolbar};
use serde::{Deserialize, Serialize};
use tao::{
    dpi::{LogicalPosition, LogicalSize},
    event::{ElementState, Event, MouseButton, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    platform::windows::WindowBuilderExtWindows,
    keyboard::ModifiersState,
    window::{CursorIcon, Theme, Window, WindowBuilder, WindowId},
};
use wry::{
    BackgroundThrottlingPolicy, MemoryUsageLevel, NewWindowResponse, PageLoadEvent,
    PermissionResponse, Rect, ScrollBarStyle, WebContext, WebView, WebViewBuilder,
    WebViewBuilderExtWindows, WebViewExtWindows,
};

/// Height of the native tab strip + toolbar, in logical pixels.
const CHROME_HEIGHT: f64 = toolbar::HEIGHT as f64;
/// Width of the draggable gap between the page and the DevTools pane.
const SPLITTER: f64 = 5.0;
const DEVTOOLS_MIN: f64 = 280.0;

const NEWTAB_HTML: &str = include_str!("ui/newtab.html");

/// Every WebView sharing a data directory must use identical arguments.
/// Tuned for fast page loads, GPU rasterization, and Wi-Fi privacy:
/// - In-process network service, async DNS, Happy Eyeballs v3, parallel downloading, back-forward
///   cache, GPU/zero-copy rasterization, and QUIC enabled.
/// - Encrypted Client Hello (ECH) enabled; WebRTC non-proxied UDP disabled to prevent local Wi-Fi IP leaks.
/// - SmartScreen, hyperlink auditing (`<a ping>`), and background telemetry disabled while keeping
///   the engine's speculative preconnect and loading predictor active.
/// - A loopback-only DevTools port chosen by the OS, which the docked DevTools pane connects to.
const BROWSER_ARGS: &str = "\
    --disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,AutofillServerCommunication,InterestFeedContentSuggestions,Translate,RendererCodeIntegrity \
    --enable-features=NetworkServiceInProcess2,AsyncDns,HappyEyeballsV3,EncryptedClientHello,BackForwardCache,ParallelDownloading,CanvasOopRasterization,UseEcoQoSForBackgroundProcess \
    --enable-quic \
    --enable-gpu-rasterization \
    --enable-zero-copy \
    --ssl-version-min=tls1.2 \
    --no-pings \
    --reduced-referrer-granularity \
    --disable-component-update \
    --disable-domain-reliability \
    --disable-client-side-phishing-detection \
    --disable-sync \
    --force-webrtc-ip-handling-policy=disable_non_proxied_udp \
    --enforce-webrtc-ip-permission-check \
    --remote-debugging-port=0";

/// Keyboard shortcuts, the context menu, speculative link preconnect/prefetch, and Wi-Fi form
/// encryption guard for content pages. Untrusted web content can only trigger the fixed `Shortcut`
/// whitelist and a validated `ContextRequest` over IPC.
const SHORTCUT_SCRIPT: &str = r#"
(() => {
  const preconnected = new Set();
  const preconnect = (raw) => {
    if (!raw || preconnected.size >= 48) return;
    try {
      const u = new URL(raw, location.href);
      if (u.protocol !== 'https:' && u.protocol !== 'http:') return;
      if (u.protocol === 'http:' && u.hostname !== 'localhost' && !u.hostname.endsWith('.localhost') && u.hostname !== '127.0.0.1') {
        u.protocol = 'https:';
      }
      const origin = u.origin;
      if (!origin || origin === 'null' || origin === location.origin || preconnected.has(origin)) return;
      const root = document.head || document.documentElement;
      if (!root) return;
      preconnected.add(origin);
      const dns = document.createElement('link');
      dns.rel = 'dns-prefetch';
      dns.href = origin;
      const pc = document.createElement('link');
      pc.rel = 'preconnect';
      pc.href = origin;
      pc.crossOrigin = 'anonymous';
      root.append(dns, pc);
    } catch (_) {}
  };
  window.__cedericPreconnect = preconnect;

  const injectSpeculation = () => {
    if (location.protocol !== 'https:' && location.protocol !== 'http:') return;
    const root = document.head || document.documentElement;
    if (!root || document.getElementById('__cederic_spec')) return;
    const s = document.createElement('script');
    s.id = '__cederic_spec';
    s.type = 'speculationrules';
    s.textContent = '{"prefetch":[{"where":{"href_matches":"https://*"},"eagerness":"moderate"}],"prerender":[{"where":{"href_matches":"https://*"},"eagerness":"conservative"}]}';
    root.appendChild(s);
  };
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', injectSpeculation, { once: true });
  } else {
    injectSpeculation();
  }

  window.addEventListener('pointerover', (e) => {
    const a = e.target instanceof Element ? e.target.closest('a[href]') : null;
    if (a && a.href) preconnect(a.href);
  }, { capture: true, passive: true });

  let ctxVideo = null;
  const videos = (root = document, out = []) => {
    for (const v of root.querySelectorAll('video')) out.push(v);
    if (out.length) return out;
    for (const el of root.querySelectorAll('*')) {
      if (el.shadowRoot) videos(el.shadowRoot, out);
    }
    return out;
  };
  const usable = (v) => v instanceof HTMLVideoElement && v.isConnected && (v.readyState >= 1 || v.videoWidth > 0 || !!(v.currentSrc || v.src));
  const videoAt = (x, y) => {
    for (const el of document.elementsFromPoint(x, y)) {
      if (usable(el)) return el;
      if (el.shadowRoot) {
        for (const v of el.shadowRoot.querySelectorAll('video')) {
          if (usable(v)) return v;
        }
      }
    }
    for (const v of document.querySelectorAll('video')) {
      if (!usable(v)) continue;
      const r = v.getBoundingClientRect();
      if (r.width > 0 && r.height > 0 && x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) return v;
    }
    return null;
  };
  const bestVideo = (preferCtx) => {
    if (preferCtx && usable(ctxVideo)) return ctxVideo;
    const list = videos().filter(usable);
    if (!list.length) return null;
    const score = (v) => {
      const r = v.getBoundingClientRect();
      const area = Math.max(0, r.width) * Math.max(0, r.height);
      const playing = (!v.paused && !v.ended) ? 1e9 : 0;
      return playing + (area || (v.videoWidth * v.videoHeight));
    };
    list.sort((a, b) => score(b) - score(a));
    return list[0];
  };
  window.__cedericTogglePip = async (preferCtx = false) => {
    try {
      if (document.pictureInPictureElement) {
        await document.exitPictureInPicture();
        window.ipc.postMessage('pip_off');
        return true;
      }
      const v = bestVideo(preferCtx);
      if (!v) return false;
      v.disablePictureInPicture = false;
      v.removeAttribute('disablepictureinpicture');
      v.addEventListener('leavepictureinpicture', () => window.ipc.postMessage('pip_off'), { once: true });
      await v.requestPictureInPicture();
      window.ipc.postMessage('pip_on');
      return true;
    } catch (_) {
      return false;
    }
  };
  window.addEventListener('enterpictureinpicture', () => window.ipc.postMessage('pip_on'), true);
  window.addEventListener('leavepictureinpicture', () => window.ipc.postMessage('pip_off'), true);
  window.addEventListener('pagehide', () => {
    if (document.pictureInPictureElement) window.ipc.postMessage('pip_off');
  }, true);

  const map = (e) => {
    const c = e.ctrlKey || e.metaKey, k = e.key.toLowerCase();
    if (c && k === 't') return 'new_tab';
    if (c && k === 'w') return 'close_tab';
    if (c && k === 'l') return 'focus_address';
    if (c && k === 'h') return 'history';
    if (c && !e.shiftKey && !e.altKey && k === 'd') return 'bookmark';
    if (c && !e.shiftKey && !e.altKey && k === 'b') return 'bookmarks';
    if (c && k === 'r' || e.key === 'F5') return 'reload';
    if (c && e.key === 'Tab') return e.shiftKey ? 'prev_tab' : 'next_tab';
    if (e.altKey && e.key === 'ArrowLeft') return 'back';
    if (e.altKey && e.key === 'ArrowRight') return 'forward';
    if (e.altKey && !c && !e.shiftKey && k === 'p') return 'pip';
    if (e.key === 'F12' || (c && e.shiftKey && k === 'i')) return 'devtools';
    return null;
  };
  window.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') window.ipc.postMessage('dismiss');
    const cmd = map(e);
    if (cmd === 'pip') {
      e.preventDefault(); e.stopPropagation();
      window.__cedericTogglePip(false).then((ok) => { if (!ok) window.ipc.postMessage('pip'); });
      return;
    }
    if (cmd) { e.preventDefault(); e.stopPropagation(); window.ipc.postMessage(cmd); return; }
    if (!e.ctrlKey && !e.altKey && !e.metaKey && (location.protocol === 'about:' || location.protocol === 'data:')) {
      const a = document.activeElement;
      const inField = a && a.closest && a.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])');
      if (!inField && (e.key.length === 1 || e.key === 'Backspace' || e.key === 'Enter')) {
        e.preventDefault(); e.stopPropagation();
        window.ipc.postMessage(JSON.stringify({ nt: 'auto_key', key: e.key }));
      }
    }
  }, true);
  // Popups never take focus, so a click in the page is what closes an open menu.
  window.addEventListener('pointerdown', (e) => {
    const a = e.target instanceof Element ? e.target.closest('a[href]') : null;
    if (a && a.href) preconnect(a.href);
    window.ipc.postMessage('dismiss');
  }, true);

  // Bubble phase: pages with their own context menu call preventDefault first and keep it.
  // Editable fields keep the engine's menu so cut/paste/spellcheck work.
  window.addEventListener('contextmenu', (e) => {
    if (e.defaultPrevented) return;
    const t = e.target instanceof Element ? e.target : null;
    if (t && t.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])')) return;
    e.preventDefault();
    const a = t && t.closest('a[href]'), img = t && t.closest('img');
    ctxVideo = videoAt(e.clientX, e.clientY) || (document.querySelector('video') && bestVideo(false)) || null;
    window.ipc.postMessage(JSON.stringify({
      x: e.clientX, y: e.clientY, dpr: window.devicePixelRatio,
      link: a ? a.href : '', image: img ? (img.currentSrc || img.src) : '',
      selection: String(getSelection()).slice(0, 20000),
      video: !!ctxVideo,
      pip: !!document.pictureInPictureElement,
    }));
  });

  // Upgrade any non-local plaintext http:// form submissions to https:// before packets leave Wi-Fi.
  window.addEventListener('submit', (e) => {
    const f = e.target;
    if (f && typeof f.action === 'string' && f.action.startsWith('http://')) {
      try {
        const u = new URL(f.action);
        if (u.hostname !== 'localhost' && !u.hostname.endsWith('.localhost') && u.hostname !== '127.0.0.1') {
          u.protocol = 'https:';
          f.action = u.toString();
        }
      } catch (_) {}
    }
  }, true);
})();
"#;

/// Shows Cederic's (or Uzaki's) name and wordmark in place of Brave's on Brave Search, and
/// preconnects top search result origins so clicking any result loads without DNS/TLS delay.
const BRAND_SCRIPT: &str = r#"
(() => {
  if (location.hostname !== 'search.brave.com') return;
  const style = document.createElement('style');
  let brandName = 'Cederic';
  const fix = (s) => s.replace(/\b(Brave|Cederic|Uzaki)\b/g, brandName);
  const fixText = (n) => {
    const p = n.parentElement;
    if (p && !/^(SCRIPT|STYLE|TEXTAREA|SVG|NOSCRIPT|CODE|PRE)$/i.test(p.tagName) && re.test(n.data)) {
      const next = fix(n.data);
      if (next !== n.data) n.data = next;
    }
  };
  const scrub = (root) => {
    if (!root) return;
    if (root.nodeType === Node.TEXT_NODE) {
      fixText(root);
      return;
    }
    const walk = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
      acceptNode: (n) => {
        const p = n.parentElement;
        return (p && /^(SCRIPT|STYLE|TEXTAREA|SVG|NOSCRIPT|CODE|PRE)$/i.test(p.tagName))
          ? NodeFilter.FILTER_REJECT
          : NodeFilter.FILTER_ACCEPT;
      }
    });
    for (let n; (n = walk.nextNode());) {
      if (re.test(n.data)) {
        const next = fix(n.data);
        if (next !== n.data) n.data = next;
      }
    }
    if (root.querySelectorAll) {
      for (const el of root.querySelectorAll('[placeholder], [title], [aria-label], [alt]')) {
        for (const a of ['placeholder', 'title', 'aria-label', 'alt']) {
          const v = el.getAttribute(a);
          if (v && re.test(v)) {
            const next = fix(v);
            if (next !== v) el.setAttribute(a, next);
          }
        }
      }
    }
  };
  const prewarmResults = () => {
    if (!window.__cedericPreconnect || !document.body) return;
    let count = 0;
    for (const a of document.body.querySelectorAll('a[href^="https://"]')) {
      if (!a.href.includes('brave.com')) {
        a.rel = 'noreferrer noopener';
        window.__cedericPreconnect(a.href);
        if (++count >= 6) break;
      }
    }
  };
  window.__cedericSetBrand = (b) => {
    const uzaki = b === 'uzaki';
    brandName = uzaki ? 'Uzaki' : 'Cederic';
    const letter = uzaki ? 'U' : 'C';
    const grad = uzaki ? 'linear-gradient(90deg, #6a5acd, #94a3f8)' : 'linear-gradient(90deg, #ff6a3d, #ffb03d)';
    const badgeGrad = uzaki ? 'linear-gradient(135deg, #6a5acd, #94a3f8)' : 'linear-gradient(135deg, #ff6a3d, #ffb03d)';
    style.textContent = `
      img[src*="brave-logo" i], img[alt*="Brave logo" i], img[alt*="Cederic logo" i], img[alt*="Uzaki logo" i],
      picture:has(img[alt*="logo" i]) source,
      .download-cta, .ask-brave-logo-divider, a[href*="//brave.com/"] { display: none !important; }
      a:has(img[src*="brave-logo" i]), a:has(img[alt*="logo" i]) { display: inline-flex !important; align-items: center;
        justify-content: center; text-decoration: none !important; }
      a:has(img[src*="brave-logo" i])::after, a:has(img[alt*="logo" i])::after { content: "${brandName}"; font: 700 24px/1 "Segoe UI Variable Display", "Segoe UI", system-ui;
        letter-spacing: -.7px; background: ${grad};
        -webkit-background-clip: text; background-clip: text; color: transparent; padding: 2px 0; }
      a.ntp-logo:has(img[src*="brave-logo" i])::after, a.ntp-logo:has(img[alt*="logo" i])::after { font-size: 64px; letter-spacing: -2.5px; padding-bottom: 6px; }
      a.logo-wrapper:has(img[src*="brave-logo" i])::after, a.logo-wrapper:has(img[alt*="logo" i])::after { content: "${letter}"; display: grid; place-items: center;
        width: 26px; height: 26px; border-radius: 8px; background: ${badgeGrad};
        -webkit-background-clip: border-box; background-clip: border-box; color: #fff; font-size: 16px;
        letter-spacing: 0; padding: 0; }`;
    if (document.body) scrub(document.body);
    if (document.title) {
      const next = fix(document.title);
      if (next !== document.title) document.title = next;
    }
  };
  window.__cedericSetBrand(window.__cedericBrand || 'cederic');
  const meta = document.createElement('meta');
  meta.name = 'referrer';
  meta.content = 'no-referrer';
  const attach = () => {
    const root = document.head || document.documentElement;
    if (root && !style.isConnected) root.appendChild(style);
    if (root && !meta.isConnected) root.appendChild(meta);
  };
  attach();
  const pending = new Set();
  let queued = false;
  new MutationObserver((mutations) => {
    attach();
    for (const m of mutations) {
      for (const n of m.addedNodes) {
        if (n !== style) pending.add(n);
      }
    }
    if (queued) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      for (const n of pending) {
        if (n.isConnected) scrub(n);
      }
      pending.clear();
      if (document.title && re.test(document.title)) {
        const next = fix(document.title);
        if (next !== document.title) document.title = next;
      }
    });
  }).observe(document, { childList: true, subtree: true });
  document.addEventListener('DOMContentLoaded', () => {
    scrub(document.body);
    prewarmResults();
  });
})();
"#;

/// DevTools pane: only the toggle shortcut is accepted.
const DEVTOOLS_SCRIPT: &str = r#"
window.addEventListener('keydown', (e) => {
  if (e.key === 'F12' || ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === 'i')) {
    e.preventDefault(); window.ipc.postMessage('devtools');
  }
}, true);
"#;

/// Finds the page's favicon; runs in the content page after it loads. The tab strip can decode
/// PNG and ICO, so those are preferred over SVG, with the site's /favicon.ico as the fallback.
const FAVICON_SCRIPT: &str = r#"(() => {
  const links = [...document.querySelectorAll('link[rel~="icon"], link[rel="shortcut icon"], link[rel="apple-touch-icon"]')];
  const raster = links.find((l) => !/svg/i.test(l.type) && !/\.svg(\?|#|$)/i.test(l.href));
  if (raster) return raster.href;
  return location.protocol.startsWith('http') ? location.origin + '/favicon.ico' : '';
})()"#;

/// Brave Search is shown under Cederic's (or Uzaki's) own name and icon.
const REAL_SEARCH_HOST: &str = "search.brave.com";
type TabId = u32;
type Rgba = (u8, u8, u8, u8);

enum UserEvent {
    Shortcut(Shortcut),
    NewTab(TabId, NewTabCmd),
    PageMenu(TabId, ContextRequest),
    Thumb(TabId, Pixmap),
    PipState(TabId, bool),
    UpgradeHttps(TabId, String),
    Title(TabId, String),
    Load(TabId, PageLoadEvent, String),
    Favicon(TabId, String),
    /// A favicon was fetched and decoded (or failed to) for the tab strip.
    IconLoaded(String, Option<Pixmap>),
    OpenInNewTab(String),
    Suggestions(suggest::Response),
    DevtoolsTarget { tab: TabId, old_title: String, url: Option<String> },
    Prewarm,
    /// Frame tick from the pacer while the UI animates.
    Frame,
}

/// What the browser chrome (the native toolbar, menus) asks the browser to do.
#[derive(Debug)]
enum ChromeCmd {
    Navigate { input: String },
    NewTab,
    CloseTab { id: TabId },
    Switch { id: TabId },
    NextTab,
    PrevTab,
    MoveTab { id: TabId, to: usize },
    TabMenu { id: TabId, x: f64, y: f64 },
    HistoryMenu { x: f64, y: f64 },
    TabHover { id: TabId, x: f64, y: f64 },
    TabHoverEnd,
    Dismiss,
    Query { text: String, x: f64, y: f64, w: f64 },
    SuggestSelect { index: usize },
    SuggestClose,
    /// The address bar took keyboard focus: move it from the page to the browser window.
    FocusChrome,
    /// Editing ended without navigating: give keyboard focus back to the page.
    FocusPage,
    Back,
    Forward,
    Reload,
    Stop,
    Devtools,
    Minimize,
    Maximize,
    CloseWindow,
    Drag,
    ClearHistory,
    Pip,
    ToggleBookmark,
    BookmarksMenu { x: f64, y: f64 },
    ThemeMenu { x: f64, y: f64 },
}

/// Messages from the built-in New Tab page (accepted only while `tab.is_newtab` is true).
#[derive(Debug, Deserialize)]
#[serde(tag = "nt", rename_all = "snake_case")]
enum NewTabCmd {
    Ready,
    Search { input: String },
    DeleteHistory { id: u64 },
    ClearHistory,
    OpenBookmark { url: String },
    DeleteBookmark { url: String },
    SetBrand { brand: Brand },
    AutoKey { key: String },
}

/// Messages from web content (untrusted): only these exact strings are accepted.
#[derive(Debug, Clone, Copy)]
enum Shortcut {
    NewTab,
    CloseTab,
    FocusAddress,
    History,
    Reload,
    NextTab,
    PrevTab,
    Back,
    Forward,
    Devtools,
    Dismiss,
    Pip,
    Bookmark,
    Bookmarks,
}

impl Shortcut {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "new_tab" => Self::NewTab,
            "close_tab" => Self::CloseTab,
            "focus_address" => Self::FocusAddress,
            "history" => Self::History,
            "reload" => Self::Reload,
            "next_tab" => Self::NextTab,
            "prev_tab" => Self::PrevTab,
            "back" => Self::Back,
            "forward" => Self::Forward,
            "devtools" => Self::Devtools,
            "dismiss" => Self::Dismiss,
            "pip" => Self::Pip,
            "bookmark" => Self::Bookmark,
            "bookmarks" => Self::Bookmarks,
            _ => return None,
        })
    }
}

/// A right-click on a page (untrusted input, sanitized in `parse`).
#[derive(Deserialize)]
struct ContextRequest {
    x: f64,
    y: f64,
    dpr: f64,
    link: String,
    image: String,
    selection: String,
    #[serde(default)]
    video: bool,
    #[serde(default)]
    pip: bool,
}

impl ContextRequest {
    fn parse(body: &str) -> Option<Self> {
        let mut r: Self = serde_json::from_str(body).ok()?;
        let web = |u: &str| u.starts_with("https://") || u.starts_with("http://");
        if !web(&r.link) {
            r.link.clear();
        }
        if !web(&r.image) {
            r.image.clear();
        }
        if !(r.x.is_finite() && r.y.is_finite() && r.dpr.is_finite() && r.dpr > 0.0) {
            return None;
        }
        Some(r)
    }
}

#[derive(Clone)]
enum MenuAction {
    NewTabRight(TabId),
    ReloadTab(TabId),
    Duplicate(TabId),
    TogglePip(TabId, bool),
    CloseTab(TabId),
    CloseOthers(TabId),
    CloseRight(TabId),
    OpenBackground(String),
    Copy(String),
    SearchFor(String),
    Back,
    Forward,
    Reload,
    ViewSource,
    Inspect,
    OpenUrl(String),
    ClearHistory,
    ToggleBookmark,
    DeleteBookmark(String),
    SetBrand(Brand),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Bookmark {
    title: String,
    url: String,
}

#[derive(Default, Serialize, Deserialize)]
struct PrefsData {
    #[serde(default)]
    brand: Brand,
    #[serde(default)]
    bookmarks: Vec<Bookmark>,
}

struct Prefs {
    path: PathBuf,
    brand: Brand,
    bookmarks: Vec<Bookmark>,
}

impl Prefs {
    fn open(data_dir: &std::path::Path) -> Self {
        let path = data_dir.join("prefs.json");
        let data: PrefsData = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, brand: data.brand, bookmarks: data.bookmarks }
    }

    fn save(&self) {
        let data = PrefsData { brand: self.brand, bookmarks: self.bookmarks.clone() };
        if let Ok(json) = serde_json::to_string_pretty(&data) {
            let tmp = self.path.with_extension("json.tmp");
            if let Some(p) = self.path.parent() {
                let _ = std::fs::create_dir_all(p);
            }
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }

    fn is_bookmarked(&self, url: &str) -> bool {
        !url.is_empty() && self.bookmarks.iter().any(|b| b.url == url)
    }

    fn toggle_bookmark(&mut self, title: &str, url: &str) -> bool {
        if url.is_empty() || url.starts_with("about:") || url.starts_with("data:") {
            return false;
        }
        if let Some(pos) = self.bookmarks.iter().position(|b| b.url == url) {
            self.bookmarks.remove(pos);
            self.save();
            false
        } else {
            let clean = title.trim();
            let t = if clean.is_empty() || clean == "New Tab" {
                host_of(url).unwrap_or(url).to_owned()
            } else {
                clean.chars().take(60).collect()
            };
            self.bookmarks.insert(0, Bookmark { title: t, url: url.to_owned() });
            if self.bookmarks.len() > 100 {
                self.bookmarks.truncate(100);
            }
            self.save();
            true
        }
    }

    fn remove_bookmark(&mut self, url: &str) -> bool {
        let before = self.bookmarks.len();
        self.bookmarks.retain(|b| b.url != url);
        if self.bookmarks.len() != before {
            self.save();
            true
        } else {
            false
        }
    }
}

struct MenuItem {
    label: String,
    hint: Option<String>,
    disabled: bool,
    sep: bool,
    action: Option<MenuAction>,
}

impl MenuItem {
    fn new(label: impl Into<String>, action: MenuAction) -> Self {
        Self { label: label.into(), hint: None, disabled: false, sep: false, action: Some(action) }
    }
    fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
    fn enabled(mut self, on: bool) -> Self {
        self.disabled = !on;
        self
    }
    fn sep() -> Self {
        Self { label: String::new(), hint: None, disabled: true, sep: true, action: None }
    }
}

struct Suggestion {
    /// Opens a site rather than searching.
    url: bool,
    text: String,
}

struct Tab {
    id: TabId,
    view: WebView,
    title: String,
    url: String,
    favicon: String,
    loading: bool,
    can_back: bool,
    can_forward: bool,
    pip: bool,
    is_newtab: bool,
    /// Still painted in the new-tab page color; switches to white once a real page loads.
    newtab_bg: bool,
    /// Small snapshot of the page for the hover card.
    thumb: Option<Pixmap>,
}

#[derive(Serialize)]
struct NewTabState<'a> {
    brand: Brand,
    bookmarks: &'a [Bookmark],
}

struct Devtools {
    view: Option<WebView>,
    open: bool,
    width: f64,
    tab: Option<TabId>,
    dragging: bool,
    hover: bool,
    seq: u64,
}

struct Browser {
    window: Rc<Window>,
    toolbar: Toolbar,
    /// Paints the toolbar strip of the main window (pages are separate child windows on top).
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    surface_size: (u32, u32),
    frame: Pixmap,
    window_title: String,
    /// When the toolbar next needs a frame (animation or timer); `None` when idle.
    next_frame: Option<Instant>,
    pacer: pacer::Pacer,
    mouse: (f64, f64),
    mods: ModifiersState,
    cursor: CursorIcon,
    /// Last laid-out window size, so repeated `Resized` events with an unchanged size (Windows sends
    /// them on focus and frame changes) don't resize every page.
    laid_out: (u32, u32, f64),
    icons_tx: Sender<String>,
    tabs: Vec<Tab>,
    /// A hidden, already-rendered new tab so Ctrl+T appears instantly.
    spare: Option<Tab>,
    active: usize,
    next_id: TabId,
    history: HistoryStore,
    prefs: Prefs,
    https_upgrades: u64,
    context: WebContext,
    data_dir: PathBuf,
    proxy: EventLoopProxy<UserEvent>,
    quit: bool,
    dark: bool,
    popups: Popups,
    menu_actions: Vec<Option<MenuAction>>,
    thumbs_tx: Sender<thumbs::Request>,
    suggest_tx: Sender<suggest::Request>,
    suggest_seq: u64,
    suggest_query: String,
    suggest_anchor: (f64, f64, f64),
    suggestions: Vec<Suggestion>,
    devtools: Devtools,
    clipboard: Option<arboard::Clipboard>,
}

impl Browser {
    fn logical_size(&self) -> LogicalSize<f64> {
        self.window.inner_size().to_logical(self.window.scale_factor())
    }

    fn top_bar_height(&self) -> f64 {
        CHROME_HEIGHT
    }

    fn devtools_width(&self) -> f64 {
        if !self.devtools.open || self.devtools.view.is_none() {
            return 0.0;
        }
        let w = self.logical_size().width;
        self.devtools.width.clamp(DEVTOOLS_MIN, (w - 300.0).max(DEVTOOLS_MIN))
    }

    fn content_bounds(&self) -> Rect {
        let size = self.logical_size();
        let top = self.top_bar_height();
        let dt = self.devtools_width();
        let w = if dt > 0.0 { size.width - dt - SPLITTER } else { size.width };
        rect(0.0, top, w.max(0.0), (size.height - top).max(0.0))
    }

    fn devtools_bounds(&self) -> Rect {
        let size = self.logical_size();
        let top = self.top_bar_height();
        let dt = self.devtools_width();
        rect(size.width - dt, top, dt, (size.height - top).max(0.0))
    }

    fn newtab_color(&self) -> Rgba {
        match (self.dark, self.prefs.brand) {
            (true, Brand::Uzaki) => (24, 29, 44, 255),
            (false, Brand::Uzaki) => (237, 240, 247, 255),
            (true, Brand::Cederic) => (29, 30, 34, 255),
            (false, Brand::Cederic) => (244, 244, 246, 255),
        }
    }

    /// Builds a hidden tab. It becomes visible when it is made the active tab.
    fn build_tab(&mut self, url: Option<&str>) -> wry::Result<Tab> {
        let id = self.next_id;
        self.next_id += 1;
        let bounds = self.content_bounds();
        let bg = self.newtab_color();
        let brand_key = if self.prefs.brand == Brand::Uzaki { "uzaki" } else { "cederic" };

        let p_title = self.proxy.clone();
        let p_load = self.proxy.clone();
        let p_new = self.proxy.clone();
        let p_ipc = self.proxy.clone();
        let p_nav = self.proxy.clone();

        let mut builder = WebViewBuilder::new_with_web_context(&mut self.context)
            .with_bounds(bounds)
            .with_visible(false)
            .with_initialization_script(format!(
                "Object.defineProperty(window, '__cedericTab', {{ value: {id} }}); window.__cedericBrand = {brand_key:?};"
            ))
            .with_initialization_script(SHORTCUT_SCRIPT)
            .with_initialization_script(BRAND_SCRIPT)
            .with_additional_browser_args(BROWSER_ARGS)
            .with_scroll_bar_style(ScrollBarStyle::FluentOverlay)
            .with_devtools(true)
            .with_hotkeys_zoom(true)
            .with_general_autofill_enabled(false)
            .with_background_throttling(BackgroundThrottlingPolicy::Suspend)
            .with_permission_handler(|_kind| PermissionResponse::Deny)
            .with_navigation_handler(move |nav_url| {
                if let Some(https_url) = input::upgrade_to_https(&nav_url) {
                    let _ = p_nav.send_event(UserEvent::UpgradeHttps(id, https_url));
                    return false;
                }
                input::is_allowed_navigation(&nav_url)
            })
            .with_document_title_changed_handler(move |t| {
                let _ = p_title.send_event(UserEvent::Title(id, t));
            })
            .with_on_page_load_handler(move |ev, url| {
                let _ = p_load.send_event(UserEvent::Load(id, ev, url));
            })
            .with_new_window_req_handler(move |url, _features| {
                let _ = p_new.send_event(UserEvent::OpenInNewTab(url));
                NewWindowResponse::Deny
            })
            .with_ipc_handler(move |req| {
                let body = req.body();
                if body == "pip_on" {
                    let _ = p_ipc.send_event(UserEvent::PipState(id, true));
                } else if body == "pip_off" {
                    let _ = p_ipc.send_event(UserEvent::PipState(id, false));
                } else if let Some(s) = Shortcut::parse(body) {
                    let _ = p_ipc.send_event(UserEvent::Shortcut(s));
                } else if let Ok(cmd) = serde_json::from_str::<NewTabCmd>(body) {
                    let _ = p_ipc.send_event(UserEvent::NewTab(id, cmd));
                } else if let Some(r) = ContextRequest::parse(body) {
                    let _ = p_ipc.send_event(UserEvent::PageMenu(id, r));
                }
            });
        builder = match url {
            Some(u) => builder.with_url(u),
            // Paint the new-tab page's own color from the first frame, so it never flashes white.
            None => builder.with_html(NEWTAB_HTML).with_background_color(bg),
        };
        let view = builder.build_as_child(&self.window)?;

        Ok(Tab {
            id,
            view,
            title: if url.is_some() { String::new() } else { "New Tab".into() },
            url: url.unwrap_or_default().to_owned(),
            favicon: String::new(),
            loading: url.is_some(),
            can_back: false,
            can_forward: false,
            pip: false,
            is_newtab: url.is_none(),
            newtab_bg: url.is_none(),
            thumb: None,
        })
    }

    /// Opens a tab at `at` (default: the end). Background tabs don't take focus.
    fn open_tab(&mut self, url: Option<String>, at: Option<usize>, foreground: bool) -> wry::Result<()> {
        let tab = match (&url, self.spare.take()) {
            (None, Some(spare)) => spare,
            (url, spare) => {
                self.spare = spare;
                self.build_tab(url.as_deref())?
            }
        };
        let at = at.unwrap_or(self.tabs.len()).min(self.tabs.len());
        let previous = self.active_tab().map(|t| t.id);
        self.tabs.insert(at, tab);
        if foreground {
            self.active = at;
            self.show_active();
            if let Some(id) = previous {
                self.hide_tab(id);
            }
            if url.is_none() {
                self.focus_address();
            }
        } else {
            if at <= self.active && self.tabs.len() > 1 {
                self.active += 1;
            }
            self.sync_ui();
        }
        // Build the next spare after this tab is on screen, so it never delays the switch.
        let _ = self.proxy.send_event(UserEvent::Prewarm);
        Ok(())
    }

    fn close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        let was_active = idx == self.active;
        if self.tabs.len() == 1 {
            // Last tab: open the replacement first so the window is never empty.
            let closing = self.tabs[idx].id;
            let _ = self.open_tab(None, None, true);
            if let Some(i) = self.index_of(closing) {
                self.close_tab(i);
            }
            return;
        }
        if was_active {
            // The right neighbour takes over (the left one at the end), shown before the closed
            // tab disappears so nothing flashes in between.
            self.active = if idx + 1 < self.tabs.len() { idx + 1 } else { idx - 1 };
            self.show_active();
        }
        // Dropping the WebView tears down its renderer and frees its memory.
        let _ = self.thumbs_tx.send(thumbs::Request::Forget(self.tabs[idx].id));
        drop(self.tabs.remove(idx));
        if idx < self.active {
            self.active -= 1;
        }
        self.sync_ui();
    }

    fn close_where(&mut self, keep: impl Fn(usize, &Tab) -> bool) {
        let kept: Vec<bool> = self.tabs.iter().enumerate().map(|(i, t)| keep(i, t)).collect();
        if !kept.contains(&true) {
            return;
        }
        // If the visible tab is closing, show the nearest kept tab before anything is dropped.
        if !kept[self.active] {
            let next = (0..self.tabs.len())
                .filter(|&i| kept[i])
                .min_by_key(|&i| i.abs_diff(self.active))
                .unwrap_or(0);
            self.active = next;
            self.show_active();
        }
        let active_id = self.tabs[self.active].id;
        for (i, t) in self.tabs.iter().enumerate() {
            if !kept[i] {
                let _ = self.thumbs_tx.send(thumbs::Request::Forget(t.id));
            }
        }
        let mut i = 0;
        self.tabs.retain(|_| {
            i += 1;
            kept[i - 1]
        });
        self.active = self.index_of(active_id).unwrap_or(0);
        self.sync_ui();
    }

    fn move_tab(&mut self, id: TabId, to: usize) {
        let Some(from) = self.index_of(id) else { return };
        let active_id = self.tabs[self.active].id;
        let tab = self.tabs.remove(from);
        self.tabs.insert(to.min(self.tabs.len()), tab);
        self.active = self.index_of(active_id).unwrap_or(0);
        self.sync_ui();
    }

    fn switch_to(&mut self, idx: usize) {
        if idx < self.tabs.len() && idx != self.active {
            let previous = self.tabs[self.active].id;
            self.active = idx;
            self.show_active();
            self.hide_tab(previous);
        }
    }

    /// Hides a background tab. Always called after the next tab is shown, never before, so the
    /// bare window never shows through for a frame.
    fn hide_tab(&self, id: TabId) {
        if let Some(t) = self.index_of(id).map(|i| &self.tabs[i]) {
            if t.pip {
                // Keep a tab with active Picture-in-Picture alive and decoding frames while
                // parked outside the visible client area, so switching tabs never freezes PiP.
                let _ = t.view.set_bounds(rect(-10.0, -10.0, 1.0, 1.0));
            } else {
                let _ = t.view.set_visible(false);
                let _ = t.view.set_memory_usage_level(MemoryUsageLevel::Low);
            }
        }
    }

    fn show_active(&mut self) {
        let bounds = self.content_bounds();
        let is_newtab = if let Some(tab) = self.tabs.get(self.active) {
            let _ = tab.view.set_memory_usage_level(MemoryUsageLevel::Normal);
            let _ = tab.view.set_bounds(bounds);
            let _ = tab.view.set_visible(true);
            if tab.is_newtab {
                Self::push_newtab_state(&tab.view, &self.prefs);
            } else {
                let _ = tab.view.focus();
            }
            let _ = self.thumbs_tx.send(thumbs::Request::Active(Some(tab.id)));
            tab.is_newtab
        } else {
            false
        };
        self.hide_preview();
        if !is_newtab {
            self.toolbar.blur_address();
        }
        self.sync_ui();
        if is_newtab {
            self.focus_address();
        }
        if self.devtools.open {
            self.attach_devtools();
        }
    }

    fn index_of(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    fn focus_address(&mut self) {
        self.toolbar.focus_address();
        self.ensure_chrome_focus();
        self.window.request_redraw();
    }

    /// Moves Win32 keyboard focus from any child WebView back to the main browser window while
    /// the address bar is being edited.
    fn ensure_chrome_focus(&self) {
        if self.toolbar.is_editing() {
            for t in &self.tabs {
                let _ = t.view.focus_parent();
            }
            if let Some(spare) = &self.spare {
                let _ = spare.view.focus_parent();
            }
            self.window.set_focus();
        }
    }

    fn push_newtab_state(view: &WebView, prefs: &Prefs) {
        let state = NewTabState {
            brand: prefs.brand,
            bookmarks: &prefs.bookmarks,
        };
        if let Ok(json) = serde_json::to_string(&state) {
            let _ = view.evaluate_script(&format!("window.renderNewTab && window.renderNewTab({json})"));
        }
    }

    fn sync_ui(&mut self) {
        let Some(tab) = self.active_tab() else { return };
        let brand = self.prefs.brand;
        let model = toolbar::Model {
            tabs: self
                .tabs
                .iter()
                .map(|t| toolbar::TabInfo {
                    id: t.id,
                    title: t.title.clone(),
                    favicon: t.favicon.clone(),
                    loading: t.loading,
                    pip: t.pip,
                })
                .collect(),
            active: tab.id,
            url: display_url(&tab.url, brand),
            can_back: tab.can_back,
            can_forward: tab.can_forward,
            loading: tab.loading,
            maximized: self.window.is_maximized(),
            devtools: self.devtools.open,
            pip: self.tabs.iter().any(|t| t.pip),
            bookmarked: self.prefs.is_bookmarked(&tab.url),
            brand,
        };
        let brand_name = brand.name();
        let title = if tab.title.is_empty() { brand_name } else { &tab.title };
        let full_title = format!("{title} - {brand_name}");
        if self.window_title != full_title {
            self.window.set_title(&full_title);
            self.window_title = full_title;
        }
        self.toolbar.set_model(model);
        for url in self.toolbar.take_icon_requests() {
            let _ = self.icons_tx.send(url);
        }
        self.window.request_redraw();
    }

    fn layout(&mut self) {
        let scale = self.window.scale_factor() as f32;
        self.toolbar.set_size(scale, self.logical_size().width as f32);
        self.window.request_redraw();
        let bounds = self.content_bounds();
        // Only resize the visible active tab (and keep any background PiP tab at its 1x1 offscreen slot);
        // hidden/suspended background tabs are resized lazily in `show_active` when switched to.
        for (i, tab) in self.tabs.iter().enumerate() {
            if i == self.active {
                let _ = tab.view.set_bounds(bounds);
            } else if tab.pip {
                let _ = tab.view.set_bounds(rect(-10.0, -10.0, 1.0, 1.0));
            }
        }
        if let Some(v) = &self.devtools.view
            && self.devtools.open
        {
            let _ = v.set_bounds(self.devtools_bounds());
        }
    }

    fn navigate_active(&mut self, raw_input: &str) {
        self.close_suggest();
        let Some(resolved) = input::resolve_with_info(&unmask(raw_input)) else {
            return;
        };
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.is_newtab = false;
            let _ = tab.view.load_url(&resolved.url);
            let _ = tab.view.focus();
        }
        self.history.record(&resolved.query, &resolved.url, resolved.engine, resolved.is_search);
        self.sync_ui();
    }

    // ---------- popups ----------

    /// Converts a point in the main window's client area to logical screen coordinates.
    fn to_screen(&self, x: f64, y: f64) -> LogicalPosition<f64> {
        let scale = self.window.scale_factor();
        let origin = self.window.inner_position().map(|p| p.to_logical::<f64>(scale)).unwrap_or_default();
        LogicalPosition::new(origin.x + x, origin.y + y)
    }

    /// Gives keyboard focus back to the page after a menu closes.
    fn refocus_page(&self) {
        self.with_active(|v| v.focus());
    }

    fn close_menu(&mut self) {
        self.popups.hide(PopupKind::Menu);
    }

    /// Shows a menu at a point in the main window's client area.
    fn show_menu(&mut self, items: Vec<MenuItem>, x: f64, y: f64) {
        const W: f64 = 248.0;
        let h = 10.0 + items.iter().map(|i| if i.sep { 9.0 } else { 30.0 }).sum::<f64>();
        let size = self.logical_size();
        let x = if x + W > size.width { (x - W).max(0.0) } else { x };
        let y = if y + h > size.height { (size.height - h).max(0.0) } else { y };
        let mut entries = Vec::with_capacity(items.len());
        self.menu_actions = items
            .into_iter()
            .map(|i| {
                entries.push(MenuEntry { label: i.label, hint: i.hint, disabled: i.disabled, sep: i.sep });
                i.action
            })
            .collect();
        self.hide_preview();
        let pos = self.to_screen(x, y);
        self.popups.show(PopupKind::Menu, Content::Menu(entries), pos, W as f32);
    }

    /// Hover card under a tab: title, site and (for background tabs) a snapshot of the page.
    fn show_preview(&mut self, id: TabId, x: f64, y: f64) {
        let w = popups::CARD_W as f64;
        if self.popups.is_open(PopupKind::Menu) {
            return;
        }
        let Some(tab) = self.index_of(id).map(|i| &self.tabs[i]) else { return };
        let active = self.active_tab().map(|t| t.id) == Some(id);
        if !active
            && let Some(cur) = self.active_tab()
        {
            let _ = self.thumbs_tx.send(thumbs::Request::Capture(cur.id));
        }
        let shown = display_url(&tab.url, self.prefs.brand);
        let card = Card {
            title: if tab.title.is_empty() { "New Tab".into() } else { tab.title.clone() },
            host: host_of(&shown).unwrap_or_default().to_owned(),
            img: if active { None } else { tab.thumb.clone() },
        };
        let x = x.clamp(0.0, (self.logical_size().width - w).max(0.0));
        let pos = self.to_screen(x, y);
        self.popups.show(PopupKind::Preview, Content::Preview(card), pos, w as f32);
    }

    fn hide_preview(&mut self) {
        self.popups.hide(PopupKind::Preview);
    }

    fn tab_menu(&mut self, id: TabId, x: f64, y: f64) {
        let Some(idx) = self.index_of(id) else { return };
        let more = self.tabs.len() > 1;
        let pip_label = if self.tabs[idx].pip { "Exit Picture in Picture" } else { "Picture in Picture" };
        let items = vec![
            MenuItem::new("New tab to the right", MenuAction::NewTabRight(id)),
            MenuItem::sep(),
            MenuItem::new("Reload", MenuAction::ReloadTab(id)).hint("Ctrl+R"),
            MenuItem::new("Duplicate", MenuAction::Duplicate(id)).enabled(!self.tabs[idx].url.is_empty()),
            MenuItem::new(pip_label, MenuAction::TogglePip(id, false))
                .hint("Alt+P")
                .enabled(!self.tabs[idx].is_newtab),
            MenuItem::sep(),
            MenuItem::new("Close tab", MenuAction::CloseTab(id)).hint("Ctrl+W"),
            MenuItem::new("Close other tabs", MenuAction::CloseOthers(id)).enabled(more),
            MenuItem::new("Close tabs to the right", MenuAction::CloseRight(id))
                .enabled(idx + 1 < self.tabs.len()),
        ];
        self.show_menu(items, x, y);
    }

    /// Recent anonymous session searches, newest first, as a dropdown under the history button.
    fn history_menu(&mut self, right: f64, y: f64) {
        const MAX: usize = 10;
        let entries = self.history.entries();
        let has_entries = !entries.is_empty();
        let mut items = vec![
            MenuItem::new("Anonymous session", MenuAction::ClearHistory)
                .hint("Auto-clears")
                .enabled(false),
            MenuItem::new("Clear search history", MenuAction::ClearHistory)
                .hint("Ctrl+H")
                .enabled(has_entries),
            MenuItem::sep(),
        ];
        if !has_entries {
            items.push(MenuItem::new("No searches in session", MenuAction::ClearHistory).enabled(false));
        } else {
            items.extend(
                entries
                    .iter()
                    .take(MAX)
                    .map(|e| MenuItem::new(e.query.clone(), MenuAction::OpenUrl(e.url.clone())).hint("Anonymous")),
            );
        }
        self.show_menu(items, (right - 248.0).max(0.0), y);
    }

    fn toggle_bookmark(&mut self) {
        let Some((title, url)) = self.active_tab().map(|t| (t.title.clone(), t.url.clone())) else {
            return;
        };
        self.prefs.toggle_bookmark(&title, &url);
        self.sync_all_newtabs();
        self.sync_ui();
    }

    fn sync_all_newtabs(&self) {
        for tab in &self.tabs {
            if tab.is_newtab {
                Self::push_newtab_state(&tab.view, &self.prefs);
            }
        }
        if let Some(spare) = &self.spare
            && spare.is_newtab
        {
            Self::push_newtab_state(&spare.view, &self.prefs);
        }
    }

    /// Saved bookmarks dropdown under the bookmarks button.
    fn bookmarks_menu(&mut self, right: f64, y: f64) {
        const MAX: usize = 14;
        let (can_bookmark, is_bookmarked, active_url) = self
            .active_tab()
            .map(|t| {
                let ok = !t.is_newtab && !t.url.is_empty() && t.url.starts_with("http");
                (ok, self.prefs.is_bookmarked(&t.url), t.url.clone())
            })
            .unwrap_or((false, false, String::new()));

        let mut items = Vec::new();
        let toggle_label = if is_bookmarked { "Remove bookmark" } else { "Bookmark this page" };
        items.push(
            MenuItem::new(toggle_label, MenuAction::ToggleBookmark)
                .hint("Ctrl+D")
                .enabled(can_bookmark),
        );
        items.push(MenuItem::sep());

        if self.prefs.bookmarks.is_empty() {
            items.push(MenuItem::new("No saved bookmarks yet", MenuAction::ToggleBookmark).enabled(false));
        } else {
            for b in self.prefs.bookmarks.iter().take(MAX) {
                let shown = display_url(&b.url, self.prefs.brand);
                let host = host_of(&shown).unwrap_or_default().to_owned();
                items.push(
                    MenuItem::new(b.title.clone(), MenuAction::OpenUrl(b.url.clone())).hint(host),
                );
            }
            if is_bookmarked && !active_url.is_empty() {
                items.push(MenuItem::sep());
                items.push(MenuItem::new(
                    "Delete current bookmark",
                    MenuAction::DeleteBookmark(active_url),
                ));
            }
        }
        self.show_menu(items, (right - 248.0).max(0.0), y);
    }

    /// Theme & persona dropdown under the palette button: switch between Cederic (Orange) and Uzaki (Slate Blue).
    fn theme_menu(&mut self, right: f64, y: f64) {
        let cur = self.prefs.brand;
        let c_check = if cur == Brand::Cederic { "✓  Cederic (Orange)" } else { "   Cederic (Orange)" };
        let u_check = if cur == Brand::Uzaki { "✓  Uzaki (Slate Blue)" } else { "   Uzaki (Slate Blue)" };
        let items = vec![
            MenuItem::new(c_check, MenuAction::SetBrand(Brand::Cederic)).hint("Default"),
            MenuItem::new(u_check, MenuAction::SetBrand(Brand::Uzaki)).hint("Slate Blue"),
        ];
        self.show_menu(items, (right - 248.0).max(0.0), y);
    }

    fn set_brand(&mut self, brand: Brand) {
        if self.prefs.brand == brand {
            return;
        }
        self.prefs.brand = brand;
        self.prefs.save();
        self.toolbar.set_theme(self.dark, brand);
        self.popups.set_theme(self.dark, brand);
        let bg = self.newtab_color();
        let brand_key = if brand == Brand::Uzaki { "uzaki" } else { "cederic" };
        let js = format!(
            "window.__cedericBrand = {brand_key:?}; window.__cedericSetBrand && window.__cedericSetBrand({brand_key:?});"
        );
        for tab in &mut self.tabs {
            if tab.newtab_bg {
                let _ = tab.view.set_background_color(bg);
            }
            if host_of(&tab.url) == Some(REAL_SEARCH_HOST) {
                tab.title = tab
                    .title
                    .replace("Brave", brand.name())
                    .replace("Cederic", brand.name())
                    .replace("Uzaki", brand.name());
            }
            let _ = tab.view.evaluate_script(&js);
        }
        if let Some(spare) = &self.spare {
            if spare.newtab_bg {
                let _ = spare.view.set_background_color(bg);
            }
            let _ = spare.view.evaluate_script(&js);
        }
        self.sync_all_newtabs();
        self.sync_ui();
    }

    fn page_menu(&mut self, id: TabId, r: ContextRequest) {
        let Some(tab) = self.active_tab().filter(|t| t.id == id) else { return };
        let mut items = Vec::new();
        if !r.link.is_empty() {
            items.push(MenuItem::new("Open link in new tab", MenuAction::OpenBackground(r.link.clone())));
            items.push(MenuItem::new("Copy link address", MenuAction::Copy(r.link)));
            items.push(MenuItem::sep());
        }
        if !r.image.is_empty() {
            items.push(MenuItem::new("Open image in new tab", MenuAction::OpenBackground(r.image.clone())));
            items.push(MenuItem::new("Copy image address", MenuAction::Copy(r.image)));
            items.push(MenuItem::sep());
        }
        if r.video || r.pip || tab.pip {
            let label = if r.pip || tab.pip { "Exit Picture in Picture" } else { "Picture in Picture" };
            items.push(MenuItem::new(label, MenuAction::TogglePip(id, true)).hint("Alt+P"));
            items.push(MenuItem::sep());
        }
        let sel = r.selection.trim();
        if !sel.is_empty() {
            let short: String = sel.chars().take(24).collect();
            let ell = if sel.chars().count() > 24 { "…" } else { "" };
            items.push(MenuItem::new("Copy", MenuAction::Copy(sel.to_owned())).hint("Ctrl+C"));
            items.push(MenuItem::new(format!("Search for “{short}{ell}”"), MenuAction::SearchFor(sel.to_owned())));
            items.push(MenuItem::sep());
        }
        let (back, fwd) = (tab.view.can_go_back().unwrap_or(false), tab.view.can_go_forward().unwrap_or(false));
        let web = tab.url.starts_with("http");
        let bm_label = if self.prefs.is_bookmarked(&tab.url) { "Remove bookmark" } else { "Bookmark this page" };
        items.push(MenuItem::new("Back", MenuAction::Back).hint("Alt+←").enabled(back));
        items.push(MenuItem::new("Forward", MenuAction::Forward).hint("Alt+→").enabled(fwd));
        items.push(MenuItem::new("Reload", MenuAction::Reload).hint("Ctrl+R"));
        items.push(MenuItem::new(bm_label, MenuAction::ToggleBookmark).hint("Ctrl+D").enabled(web));
        items.push(MenuItem::sep());
        items.push(MenuItem::new("View page source", MenuAction::ViewSource).enabled(web));
        items.push(MenuItem::new("Inspect", MenuAction::Inspect).hint("F12"));

        // Page coordinates are CSS pixels of the (possibly zoomed) page.
        let zoom = r.dpr / self.window.scale_factor();
        let top = self.top_bar_height();
        self.show_menu(items, r.x * zoom, top + r.y * zoom);
    }

    fn run_menu_action(&mut self, action: MenuAction) {
        match action {
            MenuAction::NewTabRight(id) => {
                let at = self.index_of(id).map(|i| i + 1);
                let _ = self.open_tab(None, at, true);
            }
            MenuAction::ReloadTab(id) => {
                if let Some(i) = self.index_of(id) {
                    let _ = self.tabs[i].view.reload();
                }
            }
            MenuAction::Duplicate(id) => {
                if let Some(i) = self.index_of(id) {
                    let url = self.tabs[i].url.clone();
                    let _ = self.open_tab(Some(url), Some(i + 1), true);
                }
            }
            MenuAction::TogglePip(id, prefer_ctx) => self.toggle_pip(Some(id), prefer_ctx),
            MenuAction::CloseTab(id) => {
                if let Some(i) = self.index_of(id) {
                    self.close_tab(i);
                }
            }
            MenuAction::CloseOthers(id) => self.close_where(|_, t| t.id == id),
            MenuAction::CloseRight(id) => {
                if let Some(i) = self.index_of(id) {
                    self.close_where(|j, _| j <= i);
                }
            }
            MenuAction::OpenBackground(url) => {
                let _ = self.open_tab(Some(url), Some(self.active + 1), false);
            }
            MenuAction::Copy(text) => {
                if self.clipboard.is_none() {
                    self.clipboard = arboard::Clipboard::new().ok();
                }
                if let Some(cb) = &mut self.clipboard {
                    let _ = cb.set_text(text);
                }
            }
            MenuAction::SearchFor(text) => {
                let _ = self.open_tab(Some(input::search_url(&text)), Some(self.active + 1), true);
            }
            MenuAction::Back => self.handle_chrome(ChromeCmd::Back),
            MenuAction::Forward => self.handle_chrome(ChromeCmd::Forward),
            MenuAction::Reload => self.handle_chrome(ChromeCmd::Reload),
            MenuAction::ViewSource => {
                if let Some(url) = self.active_tab().map(|t| format!("view-source:{}", t.url)) {
                    let _ = self.open_tab(Some(url), Some(self.active + 1), true);
                }
            }
            MenuAction::Inspect => {
                if !self.devtools.open {
                    self.toggle_devtools();
                }
            }
            MenuAction::OpenUrl(url) => self.navigate_active(&url),
            MenuAction::ClearHistory => self.handle_chrome(ChromeCmd::ClearHistory),
            MenuAction::ToggleBookmark => self.toggle_bookmark(),
            MenuAction::DeleteBookmark(url) => {
                if self.prefs.remove_bookmark(&url) {
                    self.sync_all_newtabs();
                    self.sync_ui();
                }
            }
            MenuAction::SetBrand(brand) => self.set_brand(brand),
        }
    }


    // ---------- search suggestions ----------

    fn query(&mut self, text: String, x: f64, y: f64, w: f64) {
        self.suggest_seq += 1;
        self.suggest_anchor = (x, y, w);
        let text = text.trim().to_owned();
        if text.is_empty() {
            self.suggestions.clear();
            self.popups.hide(PopupKind::Suggest);
            return;
        }
        // Preconnect to the target host (or search engine) while the user is still typing.
        if let Some(target_url) = input::resolve(&unmask(&text))
            && let Some(host) = host_of(&target_url)
            && let Ok(origin_js) = serde_json::to_string(&format!("https://{host}"))
        {
            self.with_active(|v| v.evaluate_script(&format!("window.__cedericPreconnect && window.__cedericPreconnect({origin_js})")));
        }
        // Show the typed row instantly, then fill in suggestions when they arrive.
        self.suggest_query = text.clone();
        self.set_suggestions(Vec::new());
        let _ = self.suggest_tx.send(suggest::Request { seq: self.suggest_seq, query: text });
    }

    fn set_suggestions(&mut self, items: Vec<String>) {
        let q = self.suggest_query.clone();
        let url = matches!(input::resolve(&unmask(&q)), Some(u) if !input::is_search(&u));
        let mut list = vec![Suggestion { url, text: q.clone() }];
        list.extend(
            items
                .into_iter()
                .filter(|s| !s.eq_ignore_ascii_case(&q))
                .map(|text| Suggestion { url: false, text }),
        );
        self.toolbar.set_suggestions(list.iter().map(|s| s.text.clone()).collect());
        let (x, y, w) = self.suggest_anchor;
        let entries = list.iter().map(|s| SuggestEntry { text: s.text.clone(), url: s.url }).collect();
        let pos = self.to_screen(x, y);
        self.popups.show(PopupKind::Suggest, Content::Suggest { items: entries, query: q }, pos, w as f32);
        self.suggestions = list;
    }

    fn close_suggest(&mut self) {
        self.suggest_seq += 1;
        self.popups.hide(PopupKind::Suggest);
    }

    // ---------- Picture-in-Picture & DevTools ----------

    fn toggle_pip(&mut self, target: Option<TabId>, prefer_ctx: bool) {
        let id = target.or_else(|| {
            let active = self.active_tab()?;
            if active.pip {
                Some(active.id)
            } else {
                self.tabs.iter().find(|t| t.pip).map(|t| t.id).or(Some(active.id))
            }
        });
        let Some(id) = id else { return };
        let Some(i) = self.index_of(id) else { return };
        if i != self.active && !self.tabs[i].pip {
            let _ = self.tabs[i].view.set_memory_usage_level(MemoryUsageLevel::Normal);
            let _ = self.tabs[i].view.set_bounds(rect(-10.0, -10.0, 1.0, 1.0));
            let _ = self.tabs[i].view.set_visible(true);
        }
        let js = format!("window.__cedericTogglePip && window.__cedericTogglePip({prefer_ctx})");
        if devtools::port(&self.data_dir).is_some() {
            let _ = self.thumbs_tx.send(thumbs::Request::EvalGesture(id, js));
        } else {
            let _ = self.tabs[i].view.evaluate_script(&js);
        }
    }

    fn toggle_devtools(&mut self) {
        self.devtools.open = !self.devtools.open;
        if self.devtools.open {
            self.attach_devtools();
        } else {
            if let Some(v) = &self.devtools.view {
                let _ = v.set_visible(false);
                let _ = v.load_url("about:blank");
            }
            self.devtools.tab = None;
            self.layout();
        }
        self.sync_ui();
    }

    /// Points the DevTools pane at the active tab. The tab is found in the engine's target list
    /// by briefly giving it a unique invisible title.
    fn attach_devtools(&mut self) {
        let Some(id) = self.active_tab().map(|t| t.id) else { return };
        if self.devtools.tab == Some(id) {
            return;
        }
        self.devtools.seq += 1;
        let marker = format!("{}cederic-{}-{}", devtools::MARKER, id, self.devtools.seq);
        let Ok(marker_js) = serde_json::to_string(&marker) else { return };
        let (dir, proxy) = (self.data_dir.clone(), self.proxy.clone());
        let js = format!("(() => {{ const t = document.title; document.title = {marker_js}; return t; }})()");
        let Some(tab) = self.active_tab() else { return };
        let page_url = tab.url.clone();
        let _ = tab.view.evaluate_script_with_callback(&js, move |old_title| {
            let (dir, marker, proxy, page_url) = (dir.clone(), marker.clone(), proxy.clone(), page_url.clone());
            std::thread::spawn(move || {
                let url = devtools::frontend_url(&dir, &marker, &page_url);
                let _ = proxy.send_event(UserEvent::DevtoolsTarget { tab: id, old_title, url });
            });
        });
    }

    fn devtools_target(&mut self, tab_id: TabId, old_title: String, url: Option<String>) {
        if let (Some(i), Ok(title)) = (self.index_of(tab_id), serde_json::from_str::<String>(&old_title))
            && let Ok(js) = serde_json::to_string(&title)
        {
            let _ = self.tabs[i].view.evaluate_script(&format!("document.title = {js}"));
        }
        if !self.devtools.open || self.active_tab().map(|t| t.id) != Some(tab_id) {
            return;
        }
        let Some(url) = url else {
            // Docking failed (e.g. the port is unavailable): fall back to the engine's window.
            self.devtools.open = false;
            if let Some(t) = self.active_tab() {
                t.view.open_devtools();
            }
            self.sync_ui();
            return;
        };
        self.devtools.tab = Some(tab_id);
        match &self.devtools.view {
            Some(v) => {
                let _ = v.load_url(&url);
                let _ = v.set_visible(true);
            }
            None => {
                let proxy = self.proxy.clone();
                self.devtools.view = WebViewBuilder::new_with_web_context(&mut self.context)
                    .with_bounds(rect(0.0, 0.0, 0.0, 0.0))
                    .with_url(&url)
                    .with_background_color(if self.dark { (36, 36, 36, 255) } else { (255, 255, 255, 255) })
                    .with_initialization_script(DEVTOOLS_SCRIPT)
                    .with_additional_browser_args(BROWSER_ARGS)
                    .with_scroll_bar_style(ScrollBarStyle::FluentOverlay)
                    .with_ipc_handler(move |req| {
                        if req.body() == "devtools" {
                            let _ = proxy.send_event(UserEvent::Shortcut(Shortcut::Devtools));
                        }
                    })
                    .build_as_child(&self.window)
                    .ok();
            }
        }
        self.layout();
    }

    /// Splitter between page and DevTools, handled on the bare window between the two WebViews.
    fn on_cursor(&mut self, x: f64, y: f64) {
        if !self.devtools.open {
            return;
        }
        let w = self.logical_size().width;
        if self.devtools.dragging {
            self.devtools.width = (w - x - SPLITTER / 2.0).max(DEVTOOLS_MIN);
            self.layout();
            return;
        }
        let edge = w - self.devtools_width();
        let hover = y > self.top_bar_height() && x >= edge - SPLITTER - 1.0 && x <= edge + 1.0;
        if hover != self.devtools.hover {
            self.devtools.hover = hover;
            self.window.set_cursor_icon(if hover { CursorIcon::ColResize } else { CursorIcon::Default });
        }
    }

    // ---------- commands ----------

    fn handle_chrome(&mut self, cmd: ChromeCmd) {
        match cmd {
            ChromeCmd::Navigate { input } => self.navigate_active(&input),
            ChromeCmd::NewTab => {
                let _ = self.open_tab(None, None, true);
            }
            ChromeCmd::CloseTab { id } => {
                if let Some(i) = self.index_of(id) {
                    self.close_tab(i);
                }
            }
            ChromeCmd::Switch { id } => {
                if let Some(i) = self.index_of(id) {
                    self.switch_to(i);
                }
            }
            ChromeCmd::NextTab => self.handle_shortcut(Shortcut::NextTab),
            ChromeCmd::PrevTab => self.handle_shortcut(Shortcut::PrevTab),
            ChromeCmd::FocusChrome => {
                self.ensure_chrome_focus();
            }
            ChromeCmd::FocusPage => self.with_active(|v| v.focus()),
            ChromeCmd::MoveTab { id, to } => self.move_tab(id, to),
            ChromeCmd::TabMenu { id, x, y } => self.tab_menu(id, x, y),
            ChromeCmd::HistoryMenu { x, y } => self.history_menu(x, y),
            ChromeCmd::TabHover { id, x, y } => self.show_preview(id, x, y),
            ChromeCmd::TabHoverEnd => self.hide_preview(),
            ChromeCmd::Dismiss => self.close_menu(),
            ChromeCmd::Query { text, x, y, w } => self.query(text, x, y, w),
            ChromeCmd::SuggestSelect { index } => {
                self.popups.select(PopupKind::Suggest, index);
            }
            ChromeCmd::SuggestClose => self.close_suggest(),
            ChromeCmd::Back => self.with_active(|v| v.go_back()),
            ChromeCmd::Forward => self.with_active(|v| v.go_forward()),
            ChromeCmd::Reload => self.with_active(|v| v.reload()),
            ChromeCmd::Stop => self.with_active(|v| v.evaluate_script("window.stop()")),
            ChromeCmd::Devtools => self.toggle_devtools(),
            ChromeCmd::Minimize => self.window.set_minimized(true),
            ChromeCmd::Maximize => self.window.set_maximized(!self.window.is_maximized()),
            ChromeCmd::CloseWindow => self.quit = true,
            ChromeCmd::Drag => {
                let _ = self.window.drag_window();
            }
            ChromeCmd::ClearHistory => {
                self.history.clear();
                for t in &self.tabs {
                    let _ = t.view.clear_all_browsing_data();
                }
                self.sync_ui();
            }
            ChromeCmd::Pip => self.toggle_pip(None, false),
            ChromeCmd::ToggleBookmark => self.toggle_bookmark(),
            ChromeCmd::BookmarksMenu { x, y } => self.bookmarks_menu(x, y),
            ChromeCmd::ThemeMenu { x, y } => self.theme_menu(x, y),
        }
    }

    fn handle_pick(&mut self, pick: Pick) {
        match pick {
            Pick::Menu(index) => {
                self.close_menu();
                self.refocus_page();
                if let Some(Some(action)) = self.menu_actions.get(index).cloned() {
                    self.run_menu_action(action);
                }
            }
            Pick::Suggest(index) => {
                if let Some(text) = self.suggestions.get(index).map(|s| s.text.clone()) {
                    self.toolbar.blur_address();
                    self.navigate_active(&text);
                }
            }
        }
    }

    fn handle_shortcut(&mut self, s: Shortcut) {
        let n = self.tabs.len();
        match s {
            Shortcut::NewTab => self.handle_chrome(ChromeCmd::NewTab),
            Shortcut::CloseTab => self.close_tab(self.active),
            Shortcut::FocusAddress => self.focus_address(),
            Shortcut::History => {
                let (x, y) = self.toolbar.history_anchor();
                self.history_menu(x as f64, y as f64);
            }
            Shortcut::Reload => self.handle_chrome(ChromeCmd::Reload),
            Shortcut::NextTab => self.switch_to((self.active + 1) % n),
            Shortcut::PrevTab => self.switch_to((self.active + n - 1) % n),
            Shortcut::Back => self.handle_chrome(ChromeCmd::Back),
            Shortcut::Forward => self.handle_chrome(ChromeCmd::Forward),
            Shortcut::Devtools => self.toggle_devtools(),
            Shortcut::Dismiss => {
                // A click or Escape in the page: close menus and stop editing the address bar.
                let active = self.popups.is_open(PopupKind::Menu) || self.toolbar.is_editing();
                self.close_menu();
                self.toolbar.blur_address();
                if active {
                    self.window.request_redraw();
                }
            }
            Shortcut::Pip => self.toggle_pip(None, false),
            Shortcut::Bookmark => self.toggle_bookmark(),
            Shortcut::Bookmarks => {
                let (x, y) = self.toolbar.bookmarks_anchor();
                self.bookmarks_menu(x as f64, y as f64);
            }
        }
    }

    fn handle_newtab(&mut self, id: TabId, cmd: NewTabCmd) {
        if let Some(spare) = &self.spare
            && spare.id == id
        {
            if matches!(cmd, NewTabCmd::Ready) {
                Self::push_newtab_state(&spare.view, &self.prefs);
            }
            return;
        }
        let Some(i) = self.index_of(id) else { return };
        if !self.tabs[i].is_newtab {
            return;
        }
        match cmd {
            NewTabCmd::Ready => {
                Self::push_newtab_state(&self.tabs[i].view, &self.prefs);
                self.ensure_chrome_focus();
            }
            NewTabCmd::AutoKey { key } => {
                if i != self.active {
                    self.switch_to(i);
                }
                self.toolbar.auto_key(&key);
                self.ensure_chrome_focus();
                self.window.request_redraw();
            }
            NewTabCmd::Search { input } => {
                if i != self.active {
                    self.switch_to(i);
                }
                self.navigate_active(&input);
            }
            NewTabCmd::DeleteHistory { id: entry_id } => {
                if self.history.remove(entry_id) {
                    self.sync_all_newtabs();
                    self.sync_ui();
                }
            }
            NewTabCmd::ClearHistory => self.handle_chrome(ChromeCmd::ClearHistory),
            NewTabCmd::OpenBookmark { url } => {
                if i != self.active {
                    self.switch_to(i);
                }
                self.navigate_active(&url);
            }
            NewTabCmd::DeleteBookmark { url } => {
                if self.prefs.remove_bookmark(&url) {
                    self.sync_all_newtabs();
                    self.sync_ui();
                }
            }
            NewTabCmd::SetBrand { brand } => self.set_brand(brand),
        }
    }

    fn with_active(&self, f: impl FnOnce(&WebView) -> wry::Result<()>) {
        if let Some(t) = self.active_tab() {
            let _ = f(&t.view);
        }
    }

    fn handle(&mut self, ev: UserEvent) {
        match ev {
            UserEvent::Shortcut(s) => self.handle_shortcut(s),
            UserEvent::NewTab(id, cmd) => self.handle_newtab(id, cmd),
            UserEvent::PageMenu(id, r) => self.page_menu(id, r),
            UserEvent::IconLoaded(url, img) => {
                self.toolbar.icon_loaded(url, img);
                self.window.request_redraw();
            }
            UserEvent::Thumb(id, img) => {
                if let Some(i) = self.index_of(id) {
                    self.tabs[i].thumb = Some(img);
                }
            }
            UserEvent::PipState(id, on) => {
                if let Some(i) = self.index_of(id) {
                    let bounds = self.content_bounds();
                    let active = self.active;
                    if on {
                        for (j, t) in self.tabs.iter_mut().enumerate() {
                            if j != i && t.pip {
                                t.pip = false;
                                if j != active {
                                    let _ = t.view.set_visible(false);
                                    let _ = t.view.set_memory_usage_level(MemoryUsageLevel::Low);
                                    let _ = t.view.set_bounds(bounds);
                                }
                            }
                        }
                    }
                    self.tabs[i].pip = on;
                    if i != active {
                        if on {
                            let _ = self.tabs[i].view.set_memory_usage_level(MemoryUsageLevel::Normal);
                            let _ = self.tabs[i].view.set_bounds(rect(-10.0, -10.0, 1.0, 1.0));
                            let _ = self.tabs[i].view.set_visible(true);
                        } else {
                            let _ = self.tabs[i].view.set_visible(false);
                            let _ = self.tabs[i].view.set_memory_usage_level(MemoryUsageLevel::Low);
                            let _ = self.tabs[i].view.set_bounds(bounds);
                        }
                    }
                    self.sync_ui();
                }
            }
            UserEvent::UpgradeHttps(id, https_url) => {
                self.https_upgrades += 1;
                if let Some((query, engine)) = input::extract_search(&https_url) {
                    self.history.record(&query, &https_url, engine, true);
                }
                if let Some(i) = self.index_of(id) {
                    self.tabs[i].is_newtab = false;
                    let _ = self.tabs[i].view.load_url(&https_url);
                }
                self.sync_ui();
            }
            UserEvent::OpenInNewTab(url) => {
                let target = if let Some(https_url) = input::upgrade_to_https(&url) {
                    self.https_upgrades += 1;
                    https_url
                } else if input::is_allowed_navigation(&url) {
                    url
                } else {
                    return;
                };
                if let Some((query, engine)) = input::extract_search(&target) {
                    self.history.record(&query, &target, engine, true);
                }
                let _ = self.open_tab(Some(target), Some(self.active + 1), true);
            }
            UserEvent::Frame => self.window.request_redraw(),
            UserEvent::Prewarm => {
                if self.spare.is_none()
                    && let Ok(spare) = self.build_tab(None)
                {
                    let _ = spare.view.set_memory_usage_level(MemoryUsageLevel::Low);
                    self.spare = Some(spare);
                    self.ensure_chrome_focus();
                }
            }
            UserEvent::Suggestions(res) => {
                if res.seq == self.suggest_seq {
                    self.set_suggestions(res.items);
                }
            }
            UserEvent::DevtoolsTarget { tab, old_title, url } => self.devtools_target(tab, old_title, url),
            UserEvent::Title(id, title) => {
                // Our own temporary DevTools marker title; the real title is restored right after.
                if title.contains(devtools::MARKER) {
                    return;
                }
                if let Some(i) = self.index_of(id) {
                    let brand_name = self.prefs.brand.name();
                    let t = &mut self.tabs[i];
                    t.title = if host_of(&t.url) == Some(REAL_SEARCH_HOST) {
                        title
                            .replace("Brave", brand_name)
                            .replace("Cederic", brand_name)
                            .replace("Uzaki", brand_name)
                    } else {
                        title
                    };
                    self.sync_ui();
                }
            }
            UserEvent::Favicon(id, icon) => {
                if let Some(i) = self.index_of(id) {
                    let t = &mut self.tabs[i];
                    t.favicon = if host_of(&t.url) == Some(REAL_SEARCH_HOST) { BRAND_ICON.into() } else { icon };
                    self.sync_ui();
                }
            }
            UserEvent::Load(id, ev, url) => {
                let Some(i) = self.index_of(id) else { return };
                let started = matches!(ev, PageLoadEvent::Started);
                if started && let Some((query, engine)) = input::extract_search(&url) {
                    self.history.record(&query, &url, engine, true);
                }
                let tab = &mut self.tabs[i];
                tab.loading = started;
                tab.can_back = tab.view.can_go_back().unwrap_or(false);
                tab.can_forward = tab.view.can_go_forward().unwrap_or(false);
                if !url.is_empty() && !url.starts_with("about:") && !url.starts_with("data:") {
                    tab.is_newtab = false;
                }
                if started && tab.newtab_bg && url.starts_with("http") {
                    tab.newtab_bg = false;
                    let _ = tab.view.set_background_color((255, 255, 255, 255));
                }
                if started && host_of(&url) != host_of(&tab.url) {
                    tab.favicon = if host_of(&url) == Some(REAL_SEARCH_HOST) {
                        BRAND_ICON.into()
                    } else {
                        String::new()
                    };
                }
                tab.url = url;
                if tab.title.is_empty() || tab.title == "New Tab" {
                    tab.title = host_of(&tab.url).unwrap_or("New Tab").to_owned();
                }
                let is_newtab = tab.is_newtab;
                let view_ref = &tab.view;
                if !started {
                    if !is_newtab {
                        let proxy = self.proxy.clone();
                        let _ = view_ref.evaluate_script_with_callback(FAVICON_SCRIPT, move |res| {
                            if let Some(icon) = parse_favicon(&res) {
                                let _ = proxy.send_event(UserEvent::Favicon(id, icon));
                            }
                        });
                    }
                    if !is_newtab && self.tabs.len() > 1 {
                        let _ = self.thumbs_tx.send(thumbs::Request::Capture(id));
                    }
                }
                if is_newtab && i == self.active {
                    self.ensure_chrome_focus();
                }
                self.sync_ui();
            }
        }
    }

    fn handle_window(&mut self, id: WindowId, event: WindowEvent) {
        if let Some(kind) = self.popups.owns(id) {
            if let Some(pick) = self.popups.window_event(kind, &event) {
                self.handle_pick(pick);
            }
            return;
        }
        if id != self.window.id() {
            return;
        }
        match event {
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                let size = self.window.inner_size();
                let now = (size.width, size.height, self.window.scale_factor());
                if now != self.laid_out {
                    self.laid_out = now;
                    self.close_suggest();
                    self.close_menu();
                    self.hide_preview();
                    self.layout();
                }
                // Maximize/restore also arrives here; refresh the toolbar either way.
                self.sync_ui();
            }
            WindowEvent::Moved(_) => {
                self.close_suggest();
                self.close_menu();
                self.hide_preview();
            }
            WindowEvent::Focused(true) => {
                self.ensure_chrome_focus();
            }
            WindowEvent::Focused(false) => {
                // Popups never take focus, so losing it means the user switched away.
                self.close_menu();
                self.hide_preview();
            }
            WindowEvent::CloseRequested => self.quit = true,
            WindowEvent::ThemeChanged(theme) => {
                self.dark = theme == Theme::Dark;
                self.toolbar.set_theme(self.dark, self.prefs.brand);
                self.popups.set_theme(self.dark, self.prefs.brand);
                let bg = self.newtab_color();
                for tab in &self.tabs {
                    if tab.newtab_bg {
                        let _ = tab.view.set_background_color(bg);
                    }
                }
                if let Some(spare) = &self.spare
                    && spare.newtab_bg
                {
                    let _ = spare.view.set_background_color(bg);
                }
                self.window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let p = position.to_logical::<f64>(self.window.scale_factor());
                self.mouse = (p.x, p.y);
                if p.y < CHROME_HEIGHT || self.toolbar.captures_mouse() {
                    let icon = self.toolbar.mouse_move(p.x as f32, p.y as f32);
                    self.set_cursor(icon);
                    self.devtools.hover = false;
                    self.window.request_redraw();
                } else {
                    self.toolbar.mouse_left();
                    self.on_cursor(p.x, p.y);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.toolbar.mouse_left();
                self.window.request_redraw();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let (x, y) = (self.mouse.0 as f32, self.mouse.1 as f32);
                match state {
                    ElementState::Pressed if self.mouse.1 < CHROME_HEIGHT => self.toolbar.mouse_down(button, x, y),
                    ElementState::Released if self.toolbar.captures_mouse() || self.mouse.1 < CHROME_HEIGHT => {
                        self.toolbar.mouse_up(button, x, y)
                    }
                    _ => {}
                }
                if button == MouseButton::Left {
                    self.devtools.dragging = state == ElementState::Pressed && self.devtools.hover;
                }
                self.window.request_redraw();
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m,
            WindowEvent::KeyboardInput { event, .. } => {
                self.toolbar.key(&event, self.mods);
                self.window.request_redraw();
            }
            WindowEvent::ReceivedImeText(text) => {
                self.toolbar.ime(&text);
                self.window.request_redraw();
            }
            _ => {}
        }
        self.flush_toolbar();
    }

    fn set_cursor(&mut self, icon: CursorIcon) {
        if icon != self.cursor {
            self.cursor = icon;
            self.window.set_cursor_icon(icon);
        }
    }

    /// Runs what the toolbar asked for since the last event.
    fn flush_toolbar(&mut self) {
        let cmds = std::mem::take(&mut self.toolbar.cmds);
        for cmd in cmds {
            self.handle_chrome(cmd);
        }
        if let Some((x, y)) = self.toolbar.caret() {
            self.window.set_ime_position(LogicalPosition::new(x as f64, y as f64));
        }
    }

    /// Draws the toolbar and shows it. Only the toolbar strip (and the gap beside the DevTools
    /// pane) is put on screen: the rest of the window is covered by page windows, and painting
    /// under them would flicker.
    fn redraw(&mut self) {
        let now = Instant::now();
        self.next_frame = self.toolbar.tick(now);
        self.flush_toolbar();
        let size = self.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        let scale = self.window.scale_factor();
        let bar_h = ((CHROME_HEIGHT * scale).round() as u32).min(h.get()).max(1);
        if self.frame.width() != w.get() || self.frame.height() != bar_h {
            let Some(pm) = Pixmap::new(w.get(), bar_h) else { return };
            self.frame = pm;
        }
        self.toolbar.render(&mut self.frame);
        let dt = self.devtools_width();
        let logical_w = self.logical_size().width;
        if self.surface_size != (w.get(), h.get()) {
            if self.surface.resize(w, h).is_err() {
                return;
            }
            self.surface_size = (w.get(), h.get());
        }
        let Ok(mut buf) = self.surface.buffer_mut() else { return };
        let width = w.get() as usize;
        let bar_px = width * bar_h as usize;
        for (d, p) in buf[..bar_px].iter_mut().zip(self.frame.data().chunks_exact(4)) {
            *d = (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
        }
        let mut damage = vec![softbuffer::Rect { x: 0, y: 0, width: w, height: NonZeroU32::new(bar_h).unwrap_or(h) }];
        if dt > 0.0 {
            // The bare strip between page and DevTools is the splitter.
            let x0 = ((logical_w - dt - SPLITTER) * scale).floor().max(0.0) as u32;
            let gap = ((SPLITTER * scale).ceil() as u32 + 1).min(w.get().saturating_sub(x0));
            let color = match (self.dark, self.prefs.brand) {
                (true, Brand::Uzaki) => 0x131722,
                (false, Brand::Uzaki) => 0xd9deec,
                (true, Brand::Cederic) => 0x18191d,
                (false, Brand::Cederic) => 0xdfe1e6,
            };
            for row in bar_h as usize..h.get() as usize {
                buf[row * width + x0 as usize..row * width + (x0 + gap) as usize].fill(color);
            }
            if let (Some(gw), Some(gh)) = (NonZeroU32::new(gap), NonZeroU32::new(h.get() - bar_h)) {
                damage.push(softbuffer::Rect { x: x0, y: bar_h, width: gw, height: gh });
            }
        }
        let _ = buf.present_with_damage(&damage);
    }
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect { position: LogicalPosition::new(x, y).into(), size: LogicalSize::new(w, h).into() }
}

/// The page reports its own favicon, so only accept web or inline image URLs.
fn parse_favicon(json: &str) -> Option<String> {
    let url: String = serde_json::from_str(json).ok()?;
    let ok = url.starts_with("https://") || url.starts_with("http://") || url.starts_with("data:image/");
    (ok && url.len() < 64 * 1024).then_some(url)
}

/// The address-bar text for `url`; Brave Search is shown as Cederic's (or Uzaki's) own search host.
fn display_url(url: &str, brand: Brand) -> String {
    if url.starts_with("about:") || url.starts_with("data:") {
        return String::new();
    }
    match url.split_once("://") {
        Some((scheme, rest)) if host_of(url) == Some(REAL_SEARCH_HOST) => {
            let shown_host = brand.search_host();
            format!("{scheme}://{shown_host}{}", &rest[REAL_SEARCH_HOST.len()..])
        }
        _ => url.to_owned(),
    }
}

/// Maps the displayed search host back to the real one when the user edits and submits it.
fn unmask(input: &str) -> String {
    input
        .replace(Brand::Cederic.search_host(), REAL_SEARCH_HOST)
        .replace(Brand::Uzaki.search_host(), REAL_SEARCH_HOST)
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then_some(host)
}

fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Cederic")
}

fn main() -> wry::Result<()> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let dir = data_dir();
    let prefs = Prefs::open(&dir);
    let brand = prefs.brand;

    let window = Rc::new(
        WindowBuilder::new()
        .with_title(brand.name())
        .with_decorations(false)
        .with_undecorated_shadow(true)
        .with_inner_size(LogicalSize::new(1280.0, 820.0))
        .with_min_inner_size(LogicalSize::new(480.0, 320.0))
        .build(&event_loop)
        .expect("failed to create window"),
    );
    let dark = window.theme() == Theme::Dark;
    // No background color on the main window: tao would fill the whole window with it whenever part
    // of it is invalidated, and since the window does not clip its children that paints over the
    // pages and makes them flash. Everything that is not a page is drawn by `Browser::redraw`.

    let mut history = HistoryStore::open(&dir);
    history.clear();
    let context = WebContext::new(Some(dir.clone()));

    let scale = window.scale_factor();
    let size = window.inner_size().to_logical::<f64>(scale);
    let toolbar = Toolbar::new_with_brand(dark, brand, scale as f32, size.width as f32);
    let softbuffer_ctx = softbuffer::Context::new(window.clone()).expect("failed to create drawing context");
    let surface = softbuffer::Surface::new(&softbuffer_ctx, window.clone()).expect("failed to create drawing surface");

    let popups = Popups::new(&event_loop, &window, dark, brand);

    let p_thumb = proxy.clone();
    let thumbs_tx = thumbs::spawn(dir.clone(), move |tab, img| {
        let _ = p_thumb.send_event(UserEvent::Thumb(tab, img));
    });

    let p_frame = proxy.clone();
    let pacer = pacer::Pacer::spawn(move || {
        let _ = p_frame.send_event(UserEvent::Frame);
    });

    let p_icon = proxy.clone();
    let icons_tx = favicons::spawn(move |url, img| {
        let _ = p_icon.send_event(UserEvent::IconLoaded(url, img));
    });

    let p_suggest = proxy.clone();
    let suggest_tx = suggest::spawn(move |res| {
        let _ = p_suggest.send_event(UserEvent::Suggestions(res));
    });
    // Warm up DNS + TLS to the suggestion server so the first keystroke is fast.
    let _ = suggest_tx.send(suggest::Request { seq: 0, query: String::new() });

    let mut browser = Browser {
        window,
        toolbar,
        surface,
        surface_size: (0, 0),
        frame: Pixmap::new(1, 1).expect("1x1 pixmap"),
        window_title: String::new(),
        next_frame: None,
        pacer,
        mouse: (-1.0, -1.0),
        mods: ModifiersState::empty(),
        cursor: CursorIcon::Default,
        laid_out: (0, 0, 0.0),
        icons_tx,
        tabs: Vec::new(),
        spare: None,
        active: 0,
        next_id: 1,
        history,
        prefs,
        https_upgrades: 0,
        context,
        data_dir: dir,
        proxy,
        quit: false,
        dark,
        popups,
        menu_actions: Vec::new(),
        thumbs_tx,
        suggest_tx,
        suggest_seq: 0,
        suggest_query: String::new(),
        suggest_anchor: (0.0, 0.0, 0.0),
        suggestions: Vec::new(),
        devtools: Devtools { view: None, open: false, width: 520.0, tab: None, dragging: false, hover: false, seq: 0 },
        clipboard: None,
    };
    let start = std::env::args().nth(1).and_then(|a| {
        let resolved = input::resolve_with_info(&unmask(&a))?;
        browser.history.record(&resolved.query, &resolved.url, resolved.engine, resolved.is_search);
        Some(resolved.url)
    });
    browser.open_tab(start, None, true)?;
    browser.layout();

    event_loop.run(move |event, _, control_flow| {
        match event {
            Event::UserEvent(ev) => {
                browser.handle(ev);
                browser.flush_toolbar();
            }
            Event::WindowEvent { window_id, event, .. } => browser.handle_window(window_id, event),
            Event::RedrawRequested(id) if id == browser.window.id() => browser.redraw(),
            Event::RedrawRequested(id) => {
                if let Some(kind) = browser.popups.owns(id) {
                    browser.popups.redraw(kind);
                }
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => browser.window.request_redraw(),
            _ => {}
        }
        // Animations are driven by the pacer (accurate frame timing); only slow timers such as the
        // caret blink or the hover-card delay use the event loop's own timeout.
        let now = Instant::now();
        let next = match (browser.next_frame, browser.popups.tick(now)) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let animating = next.is_some_and(|t| t <= now + pacer::FRAME * 3);
        browser.pacer.set(animating);
        *control_flow = match next {
            Some(t) if !animating => ControlFlow::WaitUntil(t),
            _ => ControlFlow::Wait,
        };
        if browser.quit {
            browser.history.clear();
            for t in &browser.tabs {
                let _ = t.view.clear_all_browsing_data();
            }
            *control_flow = ControlFlow::Exit;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_display_url_and_unmask() {
        let raw = "https://search.brave.com/search?q=rust";
        assert_eq!(
            display_url(raw, Brand::Cederic),
            "https://search.cederic.com/search?q=rust"
        );
        assert_eq!(
            display_url(raw, Brand::Uzaki),
            "https://search.uzaki.com/search?q=rust"
        );
        assert_eq!(unmask("https://search.cederic.com/search?q=rust"), raw);
        assert_eq!(unmask("https://search.uzaki.com/search?q=rust"), raw);
    }

    #[test]
    fn prefs_bookmark_and_brand_persistence() {
        let dir = std::env::temp_dir().join(format!(
            "cederic_prefs_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut prefs = Prefs::open(&dir);
        assert_eq!(prefs.brand, Brand::Cederic);
        assert!(prefs.bookmarks.is_empty());

        assert!(prefs.toggle_bookmark("Rust Lang", "https://www.rust-lang.org/"));
        assert!(prefs.is_bookmarked("https://www.rust-lang.org/"));
        prefs.brand = Brand::Uzaki;
        prefs.save();

        let reloaded = Prefs::open(&dir);
        assert_eq!(reloaded.brand, Brand::Uzaki);
        assert_eq!(reloaded.bookmarks.len(), 1);
        assert_eq!(reloaded.bookmarks[0].title, "Rust Lang");

        let mut reloaded = reloaded;
        assert!(!reloaded.toggle_bookmark("Rust Lang", "https://www.rust-lang.org/"));
        assert!(!reloaded.is_bookmarked("https://www.rust-lang.org/"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shortcut_parsing_includes_pip_and_bookmarks() {
        assert!(matches!(Shortcut::parse("pip"), Some(Shortcut::Pip)));
        assert!(matches!(Shortcut::parse("bookmark"), Some(Shortcut::Bookmark)));
        assert!(matches!(Shortcut::parse("bookmarks"), Some(Shortcut::Bookmarks)));
    }
}
