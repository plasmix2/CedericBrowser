#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod crypto;
mod devtools;
mod history;
mod input;
mod suggest;

use std::{
    path::PathBuf,
    sync::mpsc::Sender,
    time::{Duration, Instant},
};

use history::{HistoryEntry, HistoryStore};
use serde::{Deserialize, Serialize};
use tao::{
    dpi::{LogicalPosition, LogicalSize},
    event::{ElementState, Event, MouseButton, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    platform::windows::{WindowBuilderExtWindows, WindowExtWindows},
    window::{CursorIcon, ResizeDirection, Theme, Window, WindowBuilder, WindowId},
};
use wry::{
    BackgroundThrottlingPolicy, MemoryUsageLevel, NewWindowResponse, PageLoadEvent,
    PermissionResponse, Rect, ScrollBarStyle, WebContext, WebView, WebViewBuilder,
    WebViewBuilderExtWindows, WebViewExtWindows,
};

/// Base height of the tab strip + toolbar, in logical pixels.
const CHROME_HEIGHT: f64 = 84.0;
/// Width of the draggable gap between the page and the DevTools pane.
const SPLITTER: f64 = 5.0;
const DEVTOOLS_MIN: f64 = 280.0;

const CHROME_HTML: &str = include_str!("ui/chrome.html");
const NEWTAB_HTML: &str = include_str!("ui/newtab.html");
const POPUP_HTML: &str = include_str!("ui/popup.html");

/// Every WebView sharing a data directory must use identical arguments.
/// Hardened for Wi-Fi search privacy:
/// - Built-in Async DNS + DNS-over-HTTPS (DoH via Cloudflare) so local Wi-Fi routers cannot see DNS queries.
/// - Encrypted Client Hello (ECH) + Post-Quantum Kyber TLS key exchange + HTTPS-First automatic upgrades.
/// - WebRTC non-proxied UDP disabled to prevent local Wi-Fi IP leaks.
/// - SmartScreen, DNS prefetch, hyperlink auditing (`<a ping>`), and background telemetry disabled.
/// - A loopback-only DevTools port chosen by the OS, which the docked DevTools pane connects to.
const BROWSER_ARGS: &str = "\
    --disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,AutofillServerCommunication,InterestFeedContentSuggestions,Translate \
    --enable-features=AsyncDns,DnsOverHttps<DoHTrial,EncryptedClientHello,PostQuantumKyber,HttpsUpgrades,HttpsFirstBalancedModeAutoEnable,StrictOriginIsolation,BackForwardCache,ParallelDownloading \
    --force-fieldtrials=DoHTrial/Group1 \
    --force-fieldtrial-params=DoHTrial.Group1:server/https%3A%2F%2Fcloudflare-dns.com%2Fdns-query/method/POST \
    --enable-quic \
    --ssl-version-min=tls1.2 \
    --dns-prefetch-disable \
    --no-pings \
    --reduced-referrer-granularity \
    --disable-background-networking \
    --disable-domain-reliability \
    --disable-client-side-phishing-detection \
    --disable-sync \
    --force-webrtc-ip-handling-policy=disable_non_proxied_udp \
    --enforce-webrtc-ip-permission-check \
    --remote-debugging-port=0";

/// Keyboard shortcuts, the context menu, and Wi-Fi form/beacon encryption guard for content pages.
/// Untrusted web content can only trigger the fixed `Shortcut` whitelist and a validated
/// `ContextRequest` over IPC.
const SHORTCUT_SCRIPT: &str = r#"
(() => {
  const map = (e) => {
    const c = e.ctrlKey || e.metaKey, k = e.key.toLowerCase();
    if (c && k === 't') return 'new_tab';
    if (c && k === 'w') return 'close_tab';
    if (c && k === 'l') return 'focus_address';
    if (c && k === 'h') return 'history';
    if (c && k === 'r' || e.key === 'F5') return 'reload';
    if (c && e.key === 'Tab') return e.shiftKey ? 'prev_tab' : 'next_tab';
    if (e.altKey && e.key === 'ArrowLeft') return 'back';
    if (e.altKey && e.key === 'ArrowRight') return 'forward';
    if (e.key === 'F12' || (c && e.shiftKey && k === 'i')) return 'devtools';
    return null;
  };
  window.addEventListener('keydown', (e) => {
    const cmd = map(e);
    if (cmd) { e.preventDefault(); e.stopPropagation(); window.ipc.postMessage(cmd); }
  }, true);

  // Bubble phase: pages with their own context menu call preventDefault first and keep it.
  // Editable fields keep the engine's menu so cut/paste/spellcheck work.
  window.addEventListener('contextmenu', (e) => {
    if (e.defaultPrevented) return;
    const t = e.target instanceof Element ? e.target : null;
    if (t && t.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])')) return;
    e.preventDefault();
    const a = t && t.closest('a[href]'), img = t && t.closest('img');
    window.ipc.postMessage(JSON.stringify({
      x: e.clientX, y: e.clientY, dpr: window.devicePixelRatio,
      link: a ? a.href : '', image: img ? (img.currentSrc || img.src) : '',
      selection: String(getSelection()).slice(0, 20000),
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

/// Shows Cederic's name and wordmark in place of Brave's on Brave Search.
const BRAND_SCRIPT: &str = r#"
(() => {
  if (location.hostname !== 'search.brave.com') return;
  const css = `
    img[alt="Brave logo"], .download-cta { display: none !important; }
    a.ntp-logo, a.nav-logo, a.logo-wrapper { display: inline-flex !important; align-items: center;
      justify-content: center; text-decoration: none !important; }
    a.ntp-logo::after, a.nav-logo::after { content: "Cederic"; font: 700 26px/1 "Segoe UI Variable Display", "Segoe UI", system-ui;
      letter-spacing: -.8px; background: linear-gradient(90deg, #ff6a3d, #ffb03d);
      -webkit-background-clip: text; background-clip: text; color: transparent; }
    a.ntp-logo::after { font-size: 64px; letter-spacing: -2.5px; padding-bottom: 6px; }
    a.logo-wrapper::after { content: "C"; display: grid; place-items: center; width: 26px; height: 26px;
      border-radius: 8px; background: linear-gradient(135deg, #ff6a3d, #ffb03d); color: #fff;
      font: 700 16px/1 "Segoe UI Variable Display", "Segoe UI", system-ui; }`;
  const style = document.createElement('style');
  style.textContent = css;
  // Rename "Brave" in visible text and labels, but never inside what the user typed.
  const re = /\bBrave\b/;
  const fix = (s) => s.replace(/\bBrave\b/g, 'Cederic');
  const fixText = (n) => {
    const p = n.parentElement;
    if (p && !/^(SCRIPT|STYLE|TEXTAREA)$/.test(p.tagName) && re.test(n.data)) n.data = fix(n.data);
  };
  const scrub = (root) => {
    const walk = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    for (let n; (n = walk.nextNode());) fixText(n);
    for (const el of root.querySelectorAll('[placeholder], [title], [aria-label], [alt]')) {
      for (const a of ['placeholder', 'title', 'aria-label', 'alt']) {
        const v = el.getAttribute(a);
        if (v && re.test(v)) el.setAttribute(a, fix(v));
      }
    }
  };
  // Runs before the document has a root element, so keep re-attaching until (and after) it exists.
  const attach = () => {
    const root = document.head || document.documentElement;
    if (root && !style.isConnected) root.appendChild(style);
  };
  attach();
  new MutationObserver((muts) => {
    attach();
    for (const m of muts) {
      if (m.type === 'characterData') fixText(m.target);
      else for (const n of m.addedNodes) {
        if (n.nodeType === 3) fixText(n);
        else if (n.nodeType === 1) scrub(n);
      }
    }
  }).observe(document, { childList: true, subtree: true, characterData: true });
  document.addEventListener('DOMContentLoaded', () => scrub(document.body));
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

/// Finds the page's favicon; runs in the content page after it loads.
const FAVICON_SCRIPT: &str = r#"(() => {
  const l = document.querySelector('link[rel~="icon"], link[rel="shortcut icon"], link[rel="apple-touch-icon"]');
  return l ? l.href : (location.protocol.startsWith('http') ? location.origin + '/favicon.ico' : '');
})()"#;

/// Brave Search is shown under Cederic's own name and icon.
const REAL_SEARCH_HOST: &str = "search.brave.com";
const SHOWN_SEARCH_HOST: &str = "search.cederic.com";
const CEDERIC_ICON: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'%3E\
%3Cdefs%3E%3ClinearGradient id='g' x1='0' y1='0' x2='1' y2='1'%3E%3Cstop offset='0' stop-color='%23ff6a3d'/%3E\
%3Cstop offset='1' stop-color='%23ffb03d'/%3E%3C/linearGradient%3E%3C/defs%3E\
%3Crect width='32' height='32' rx='9' fill='url(%23g)'/%3E%3Ctext x='16' y='23' font-family='Segoe UI,sans-serif' \
font-size='20' font-weight='700' fill='white' text-anchor='middle'%3EC%3C/text%3E%3C/svg%3E";

/// Where never-hidden popups wait while they are not in use.
const PARKED: LogicalPosition<f64> = LogicalPosition::new(-32000.0, -32000.0);

type TabId = u32;
type Rgba = (u8, u8, u8, u8);

enum UserEvent {
    Chrome(ChromeCmd),
    Shortcut(Shortcut),
    NewTab(TabId, NewTabCmd),
    PageMenu(TabId, ContextRequest),
    /// A popup message, and whether it came from the menu (vs. the suggestions popup).
    Popup(PopupCmd, bool),
    UpgradeHttps(TabId, String),
    Title(TabId, String),
    Load(TabId, PageLoadEvent, String),
    Favicon(TabId, String),
    OpenInNewTab(String),
    Suggestions(suggest::Response),
    DevtoolsTarget { tab: TabId, old_title: String, url: Option<String> },
    Prewarm,
}

/// Messages from the browser UI (trusted, our own HTML).
#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum ChromeCmd {
    Navigate { input: String },
    NewTab,
    CloseTab { id: TabId },
    Switch { id: TabId },
    MoveTab { id: TabId, to: usize },
    TabMenu { id: TabId, x: f64, y: f64 },
    Query { text: String, x: f64, y: f64, w: f64 },
    SuggestSelect { index: usize },
    SuggestClose,
    Back,
    Forward,
    Reload,
    Stop,
    Devtools,
    Ready,
    Minimize,
    Maximize,
    CloseWindow,
    Drag,
    ResizeTop,
    SetPanel { height: f64 },
    DeleteHistory { id: u64 },
    ClearHistory,
}

/// Messages from the built-in New Tab page (accepted only while `tab.is_newtab` is true).
#[derive(Debug, Deserialize)]
#[serde(tag = "nt", rename_all = "snake_case")]
enum NewTabCmd {
    Ready,
    Search { input: String },
    DeleteHistory { id: u64 },
    ClearHistory,
}

/// Messages from our popup page.
#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum PopupCmd {
    Pick { index: usize },
    Close,
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
}

#[derive(Serialize)]
struct MenuItem {
    label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<&'static str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    disabled: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    sep: bool,
    #[serde(skip)]
    action: Option<MenuAction>,
}

impl MenuItem {
    fn new(label: impl Into<String>, action: MenuAction) -> Self {
        Self { label: label.into(), hint: None, disabled: false, sep: false, action: Some(action) }
    }
    fn hint(mut self, hint: &'static str) -> Self {
        self.hint = Some(hint);
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

#[derive(Serialize)]
struct Suggestion {
    kind: &'static str,
    text: String,
}

struct Tab {
    id: TabId,
    view: WebView,
    title: String,
    url: String,
    favicon: String,
    loading: bool,
    is_newtab: bool,
    /// Still painted in the new-tab page color; switches to white once a real page loads.
    newtab_bg: bool,
}

#[derive(Serialize)]
struct TabState<'a> {
    id: TabId,
    title: &'a str,
    favicon: &'a str,
    loading: bool,
}

#[derive(Serialize)]
struct UiState<'a> {
    tabs: Vec<TabState<'a>>,
    active: TabId,
    url: &'a str,
    can_back: bool,
    can_forward: bool,
    loading: bool,
    maximized: bool,
    devtools: bool,
    history: &'a [HistoryEntry],
    https_upgrades: u64,
}

#[derive(Serialize)]
struct NewTabState<'a> {
    history: &'a [HistoryEntry],
}

struct Popup {
    window: Window,
    view: WebView,
    /// Never-hidden popups are parked off-screen instead: tao can only show a window without
    /// activating it at creation, and the suggestions must not steal the address bar's focus.
    parked: bool,
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
    window: Window,
    chrome: WebView,
    tabs: Vec<Tab>,
    /// A hidden, already-rendered new tab so Ctrl+T appears instantly.
    spare: Option<Tab>,
    active: usize,
    next_id: TabId,
    panel_height: f64,
    history: HistoryStore,
    https_upgrades: u64,
    context: WebContext,
    data_dir: PathBuf,
    proxy: EventLoopProxy<UserEvent>,
    quit: bool,
    dark: bool,
    menu: Popup,
    menu_actions: Vec<Option<MenuAction>>,
    menu_shown: Instant,
    suggest: Popup,
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
        CHROME_HEIGHT + self.panel_height
    }

    fn chrome_bounds(&self) -> Rect {
        rect(0.0, 0.0, self.logical_size().width, self.top_bar_height())
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
        if self.dark { (29, 30, 34, 255) } else { (244, 244, 246, 255) }
    }

    /// Builds a hidden tab. It becomes visible when it is made the active tab.
    fn build_tab(&mut self, url: Option<&str>) -> wry::Result<Tab> {
        let id = self.next_id;
        self.next_id += 1;
        let bounds = self.content_bounds();
        let bg = self.newtab_color();

        let p_title = self.proxy.clone();
        let p_load = self.proxy.clone();
        let p_new = self.proxy.clone();
        let p_ipc = self.proxy.clone();
        let p_nav = self.proxy.clone();

        let mut builder = WebViewBuilder::new_with_web_context(&mut self.context)
            .with_bounds(bounds)
            .with_visible(false)
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
                if let Some(s) = Shortcut::parse(body) {
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
            is_newtab: url.is_none(),
            newtab_bg: url.is_none(),
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
        if foreground {
            self.hide_active();
        }
        self.tabs.insert(at, tab);
        if foreground {
            self.active = at;
            self.show_active();
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
        // Dropping the WebView tears down its renderer and frees its memory.
        drop(self.tabs.remove(idx));
        if self.tabs.is_empty() {
            self.active = 0;
            let _ = self.open_tab(None, None, true);
            return;
        }
        if idx < self.active || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1).min(self.tabs.len() - 1);
        }
        if was_active {
            self.show_active();
        } else {
            self.sync_ui();
        }
    }

    fn close_where(&mut self, keep: impl Fn(usize, &Tab) -> bool) {
        let active_id = self.active_tab().map(|t| t.id);
        let mut i = 0;
        self.tabs.retain(|t| {
            let k = keep(i, t);
            i += 1;
            k
        });
        self.active = active_id
            .and_then(|id| self.index_of(id))
            .unwrap_or(0)
            .min(self.tabs.len().saturating_sub(1));
        self.show_active();
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
            self.hide_active();
            self.active = idx;
            self.show_active();
        }
    }

    fn hide_active(&self) {
        if let Some(cur) = self.tabs.get(self.active) {
            let _ = cur.view.set_visible(false);
            let _ = cur.view.set_memory_usage_level(MemoryUsageLevel::Low);
        }
    }

    fn show_active(&mut self) {
        let bounds = self.content_bounds();
        if let Some(tab) = self.tabs.get(self.active) {
            let _ = tab.view.set_memory_usage_level(MemoryUsageLevel::Normal);
            let _ = tab.view.set_bounds(bounds);
            let _ = tab.view.set_visible(true);
            let _ = tab.view.focus();
        }
        self.sync_ui();
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

    fn focus_address(&self) {
        let _ = self.chrome.focus();
        let _ = self.chrome.evaluate_script("window.focusAddress && window.focusAddress()");
    }

    fn push_newtab_state(view: &WebView, history: &HistoryStore) {
        let state = NewTabState { history: history.entries() };
        if let Ok(json) = serde_json::to_string(&state) {
            let _ = view.evaluate_script(&format!("window.renderNewTab && window.renderNewTab({json})"));
        }
    }

    fn sync_ui(&self) {
        let Some(tab) = self.active_tab() else { return };
        let url = display_url(&tab.url);
        let state = UiState {
            tabs: self
                .tabs
                .iter()
                .map(|t| TabState { id: t.id, title: &t.title, favicon: &t.favicon, loading: t.loading })
                .collect(),
            active: tab.id,
            url: &url,
            can_back: tab.view.can_go_back().unwrap_or(false),
            can_forward: tab.view.can_go_forward().unwrap_or(false),
            loading: tab.loading,
            maximized: self.window.is_maximized(),
            devtools: self.devtools.open,
            history: self.history.entries(),
            https_upgrades: self.https_upgrades,
        };
        if let Ok(json) = serde_json::to_string(&state) {
            let _ = self.chrome.evaluate_script(&format!("window.render && window.render({json})"));
        }
        if tab.is_newtab {
            Self::push_newtab_state(&tab.view, &self.history);
        }
        let title = if tab.title.is_empty() { "Cederic" } else { &tab.title };
        self.window.set_title(&format!("{title} - Cederic"));
    }

    fn layout(&self) {
        let _ = self.chrome.set_bounds(self.chrome_bounds());
        let bounds = self.content_bounds();
        for tab in self.tabs.iter().chain(&self.spare) {
            let _ = tab.view.set_bounds(bounds);
        }
        if let Some(v) = &self.devtools.view {
            let _ = v.set_bounds(self.devtools_bounds());
        }
    }

    fn navigate_active(&mut self, raw_input: &str) {
        self.close_suggest();
        let Some(resolved) = input::resolve_with_info(&unmask(raw_input)) else {
            return;
        };
        self.history.record(&resolved.query, &resolved.url, resolved.engine, resolved.is_search);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.is_newtab = false;
            let _ = tab.view.load_url(&resolved.url);
            let _ = tab.view.focus();
        }
        self.sync_ui();
    }

    // ---------- popups ----------

    /// Converts a point in the main window's client area to logical screen coordinates.
    fn to_screen(&self, x: f64, y: f64) -> LogicalPosition<f64> {
        let scale = self.window.scale_factor();
        let origin = self.window.inner_position().map(|p| p.to_logical::<f64>(scale)).unwrap_or_default();
        LogicalPosition::new(origin.x + x, origin.y + y)
    }

    fn show_popup(popup: &Popup, script: &str, pos: LogicalPosition<f64>, w: f64, h: f64) {
        let _ = popup.view.evaluate_script(script);
        popup.window.set_inner_size(LogicalSize::new(w, h));
        popup.window.set_outer_position(pos);
        let _ = popup.view.set_bounds(rect(0.0, 0.0, w, h));
        if !popup.parked {
            popup.window.set_visible(true);
        }
    }

    fn hide_popup(popup: &Popup) {
        let _ = popup.view.evaluate_script("window.hideAll && window.hideAll()");
        if popup.parked {
            popup.window.set_outer_position(PARKED);
        } else {
            popup.window.set_visible(false);
        }
    }

    /// Shows a menu at a point in the main window's client area.
    fn show_menu(&mut self, items: Vec<MenuItem>, x: f64, y: f64) {
        const W: f64 = 248.0;
        let h = 10.0 + items.iter().map(|i| if i.sep { 9.0 } else { 30.0 }).sum::<f64>();
        let size = self.logical_size();
        let x = if x + W > size.width { (x - W).max(0.0) } else { x };
        let y = if y + h > size.height { (size.height - h).max(0.0) } else { y };
        let Ok(json) = serde_json::to_string(&items) else { return };
        self.menu_actions = items.into_iter().map(|i| i.action).collect();
        Self::show_popup(&self.menu, &format!("window.showMenu({json})"), self.to_screen(x, y), W, h);
        self.menu_shown = Instant::now();
        self.menu.window.set_focus();
        let _ = self.menu.view.focus();
    }

    fn tab_menu(&mut self, id: TabId, x: f64, y: f64) {
        let Some(idx) = self.index_of(id) else { return };
        let more = self.tabs.len() > 1;
        let items = vec![
            MenuItem::new("New tab to the right", MenuAction::NewTabRight(id)),
            MenuItem::sep(),
            MenuItem::new("Reload", MenuAction::ReloadTab(id)).hint("Ctrl+R"),
            MenuItem::new("Duplicate", MenuAction::Duplicate(id)).enabled(!self.tabs[idx].url.is_empty()),
            MenuItem::sep(),
            MenuItem::new("Close tab", MenuAction::CloseTab(id)).hint("Ctrl+W"),
            MenuItem::new("Close other tabs", MenuAction::CloseOthers(id)).enabled(more),
            MenuItem::new("Close tabs to the right", MenuAction::CloseRight(id))
                .enabled(idx + 1 < self.tabs.len()),
        ];
        self.show_menu(items, x, y);
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
        let sel = r.selection.trim();
        if !sel.is_empty() {
            let short: String = sel.chars().take(24).collect();
            let ell = if sel.chars().count() > 24 { "…" } else { "" };
            items.push(MenuItem::new("Copy", MenuAction::Copy(sel.to_owned())).hint("Ctrl+C"));
            items.push(MenuItem::new(format!("Search for “{short}{ell}”"), MenuAction::SearchFor(sel.to_owned())));
            items.push(MenuItem::sep());
        }
        let (back, fwd) = (tab.view.can_go_back().unwrap_or(false), tab.view.can_go_forward().unwrap_or(false));
        items.push(MenuItem::new("Back", MenuAction::Back).hint("Alt+←").enabled(back));
        items.push(MenuItem::new("Forward", MenuAction::Forward).hint("Alt+→").enabled(fwd));
        items.push(MenuItem::new("Reload", MenuAction::Reload).hint("Ctrl+R"));
        items.push(MenuItem::sep());
        let web = tab.url.starts_with("http");
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
        }
    }

    /// Gives keyboard focus back to the page after a menu closes.
    fn refocus_page(&self) {
        self.window.set_focus();
        self.with_active(|v| v.focus());
    }

    // ---------- search suggestions ----------

    fn query(&mut self, text: String, x: f64, y: f64, w: f64) {
        self.suggest_seq += 1;
        self.suggest_anchor = (x, y, w);
        let text = text.trim().to_owned();
        if text.is_empty() {
            self.suggestions.clear();
            Self::hide_popup(&self.suggest);
            return;
        }
        // Show the typed row instantly, then fill in suggestions when they arrive.
        self.suggest_query = text.clone();
        self.set_suggestions(Vec::new());
        let _ = self.suggest_tx.send(suggest::Request { seq: self.suggest_seq, query: text });
    }

    fn set_suggestions(&mut self, items: Vec<String>) {
        let q = &self.suggest_query;
        let kind = match input::resolve(&unmask(q)) {
            Some(url) if !input::is_search(&url) => "url",
            _ => "search",
        };
        let mut list = vec![Suggestion { kind, text: q.clone() }];
        list.extend(
            items
                .into_iter()
                .filter(|s| !s.eq_ignore_ascii_case(q))
                .map(|text| Suggestion { kind: "suggest", text }),
        );
        let Ok(json) = serde_json::to_string(&list) else { return };
        let _ = self.chrome.evaluate_script(&format!("window.setSuggestions && window.setSuggestions({json})"));
        let (x, y, w) = self.suggest_anchor;
        let h = 12.0 + 36.0 * list.len() as f64;
        Self::show_popup(&self.suggest, &format!("window.showSuggest({json})"), self.to_screen(x, y), w, h);
        self.suggestions = list;
    }

    fn close_suggest(&mut self) {
        self.suggest_seq += 1;
        Self::hide_popup(&self.suggest);
    }

    // ---------- DevTools ----------

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
            ChromeCmd::Ready => self.sync_ui(),
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
            ChromeCmd::MoveTab { id, to } => self.move_tab(id, to),
            ChromeCmd::TabMenu { id, x, y } => self.tab_menu(id, x, y),
            ChromeCmd::Query { text, x, y, w } => self.query(text, x, y, w),
            ChromeCmd::SuggestSelect { index } => {
                let _ = self.suggest.view.evaluate_script(&format!("window.select({index})"));
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
            ChromeCmd::ResizeTop => {
                let _ = self.window.drag_resize_window(ResizeDirection::North);
            }
            ChromeCmd::SetPanel { height } => {
                let clamped = height.clamp(0.0, 480.0);
                if (self.panel_height - clamped).abs() > 0.5 {
                    self.panel_height = clamped;
                    self.layout();
                }
            }
            ChromeCmd::DeleteHistory { id } => {
                if self.history.remove(id) {
                    self.sync_ui();
                }
            }
            ChromeCmd::ClearHistory => {
                self.history.clear();
                if let Some(t) = self.active_tab() {
                    let _ = t.view.clear_all_browsing_data();
                }
                self.sync_ui();
            }
        }
    }

    fn handle_popup(&mut self, cmd: PopupCmd, from_menu: bool) {
        match (cmd, from_menu) {
            (PopupCmd::Close, true) => {
                Self::hide_popup(&self.menu);
                self.refocus_page();
            }
            (PopupCmd::Close, false) => self.close_suggest(),
            (PopupCmd::Pick { index }, true) => {
                Self::hide_popup(&self.menu);
                self.refocus_page();
                if let Some(Some(action)) = self.menu_actions.get(index).cloned() {
                    self.run_menu_action(action);
                }
            }
            (PopupCmd::Pick { index }, false) => {
                if let Some(text) = self.suggestions.get(index).map(|s| s.text.clone()) {
                    let _ = self.chrome.evaluate_script("window.blurAddress && window.blurAddress()");
                    self.window.set_focus();
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
                let _ = self.chrome.focus();
                let _ = self.chrome.evaluate_script("window.toggleHistoryDrawer && window.toggleHistoryDrawer()");
            }
            Shortcut::Reload => self.handle_chrome(ChromeCmd::Reload),
            Shortcut::NextTab => self.switch_to((self.active + 1) % n),
            Shortcut::PrevTab => self.switch_to((self.active + n - 1) % n),
            Shortcut::Back => self.handle_chrome(ChromeCmd::Back),
            Shortcut::Forward => self.handle_chrome(ChromeCmd::Forward),
            Shortcut::Devtools => self.toggle_devtools(),
        }
    }

    fn handle_newtab(&mut self, id: TabId, cmd: NewTabCmd) {
        if let Some(spare) = &self.spare
            && spare.id == id
        {
            if matches!(cmd, NewTabCmd::Ready) {
                Self::push_newtab_state(&spare.view, &self.history);
            }
            return;
        }
        let Some(i) = self.index_of(id) else { return };
        if !self.tabs[i].is_newtab {
            return;
        }
        match cmd {
            NewTabCmd::Ready => Self::push_newtab_state(&self.tabs[i].view, &self.history),
            NewTabCmd::Search { input } => {
                if i != self.active {
                    self.switch_to(i);
                }
                self.navigate_active(&input);
            }
            NewTabCmd::DeleteHistory { id: entry_id } => {
                if self.history.remove(entry_id) {
                    self.sync_ui();
                }
            }
            NewTabCmd::ClearHistory => self.handle_chrome(ChromeCmd::ClearHistory),
        }
    }

    fn with_active(&self, f: impl FnOnce(&WebView) -> wry::Result<()>) {
        if let Some(t) = self.active_tab() {
            let _ = f(&t.view);
        }
    }

    fn handle(&mut self, ev: UserEvent) {
        match ev {
            UserEvent::Chrome(cmd) => self.handle_chrome(cmd),
            UserEvent::Shortcut(s) => self.handle_shortcut(s),
            UserEvent::NewTab(id, cmd) => self.handle_newtab(id, cmd),
            UserEvent::PageMenu(id, r) => self.page_menu(id, r),
            UserEvent::Popup(cmd, from_menu) => self.handle_popup(cmd, from_menu),
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
            UserEvent::Prewarm => {
                if self.spare.is_none() {
                    self.spare = self.build_tab(None).ok();
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
                    let t = &mut self.tabs[i];
                    t.title = if host_of(&t.url) == Some(REAL_SEARCH_HOST) {
                        title.replace("Brave", "Cederic")
                    } else {
                        title
                    };
                    self.sync_ui();
                }
            }
            UserEvent::Favicon(id, icon) => {
                if let Some(i) = self.index_of(id) {
                    let t = &mut self.tabs[i];
                    t.favicon = if host_of(&t.url) == Some(REAL_SEARCH_HOST) { CEDERIC_ICON.into() } else { icon };
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
                if !url.is_empty() && !url.starts_with("about:") && !url.starts_with("data:") {
                    tab.is_newtab = false;
                }
                if started && tab.newtab_bg && url.starts_with("http") {
                    tab.newtab_bg = false;
                    let _ = tab.view.set_background_color((255, 255, 255, 255));
                }
                if started && host_of(&url) != host_of(&tab.url) {
                    tab.favicon = if host_of(&url) == Some(REAL_SEARCH_HOST) {
                        CEDERIC_ICON.into()
                    } else {
                        String::new()
                    };
                }
                tab.url = url;
                if tab.title.is_empty() || tab.title == "New Tab" {
                    tab.title = host_of(&tab.url).unwrap_or("New Tab").to_owned();
                }
                if !started {
                    let proxy = self.proxy.clone();
                    let _ = tab.view.evaluate_script_with_callback(FAVICON_SCRIPT, move |res| {
                        if let Some(icon) = parse_favicon(&res) {
                            let _ = proxy.send_event(UserEvent::Favicon(id, icon));
                        }
                    });
                }
                self.sync_ui();
            }
        }
    }

    fn handle_window(&mut self, id: WindowId, event: WindowEvent) {
        if id == self.menu.window.id() {
            if let WindowEvent::Focused(false) = event {
                // Right after a right-click the page grabs focus back once; keep the menu up
                // and take focus again instead of closing it.
                if self.menu_shown.elapsed() < Duration::from_millis(400) {
                    self.menu.window.set_focus();
                    let _ = self.menu.view.focus();
                } else {
                    Self::hide_popup(&self.menu);
                }
            }
            return;
        }
        if id == self.suggest.window.id() {
            // The suggestions popup must never take focus from the address bar.
            if let WindowEvent::Focused(true) = event {
                self.window.set_focus();
                let _ = self.chrome.focus();
            }
            return;
        }
        match event {
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                self.close_suggest();
                self.layout();
                self.sync_ui();
            }
            WindowEvent::Moved(_) => self.close_suggest(),
            WindowEvent::CloseRequested => self.quit = true,
            WindowEvent::CursorMoved { position, .. } => {
                let p = position.to_logical::<f64>(self.window.scale_factor());
                self.on_cursor(p.x, p.y);
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                self.devtools.dragging = state == ElementState::Pressed && self.devtools.hover;
            }
            _ => {}
        }
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

/// The address-bar text for `url`; Brave Search is shown as Cederic's own search host.
fn display_url(url: &str) -> String {
    if url.starts_with("about:") || url.starts_with("data:") {
        return String::new();
    }
    match url.split_once("://") {
        Some((scheme, rest)) if host_of(url) == Some(REAL_SEARCH_HOST) => {
            format!("{scheme}://{SHOWN_SEARCH_HOST}{}", &rest[REAL_SEARCH_HOST.len()..])
        }
        _ => url.to_owned(),
    }
}

/// Maps the displayed search host back to the real one when the user edits and submits it.
fn unmask(input: &str) -> String {
    input.replace(SHOWN_SEARCH_HOST, REAL_SEARCH_HOST)
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then_some(host)
}

fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Cederic")
}

/// A borderless owned popup window (menus, suggestions) hosting our popup page. Windows 11 gives
/// it a native drop shadow and rounded corners.
fn build_popup(
    target: &EventLoopWindowTarget<UserEvent>,
    owner: &Window,
    context: &mut WebContext,
    proxy: EventLoopProxy<UserEvent>,
    is_menu: bool,
    dark: bool,
) -> wry::Result<Popup> {
    let card: Rgba = if dark { (43, 44, 50, 255) } else { (255, 255, 255, 255) };
    let window = WindowBuilder::new()
        .with_decorations(false)
        .with_undecorated_shadow(true)
        .with_background_color(card)
        .with_resizable(false)
        .with_visible(!is_menu)
        .with_focused(false)
        .with_focusable(is_menu)
        .with_position(PARKED)
        .with_skip_taskbar(true)
        .with_owner_window(owner.hwnd())
        .with_inner_size(LogicalSize::new(10.0, 10.0))
        .build(target)
        .expect("failed to create popup window");
    let view = WebViewBuilder::new_with_web_context(context)
        .with_html(POPUP_HTML)
        .with_background_color(card)
        .with_additional_browser_args(BROWSER_ARGS)
        .with_scroll_bar_style(ScrollBarStyle::FluentOverlay)
        .with_hotkeys_zoom(false)
        .with_ipc_handler(move |req| {
            if let Ok(cmd) = serde_json::from_str::<PopupCmd>(req.body()) {
                let _ = proxy.send_event(UserEvent::Popup(cmd, is_menu));
            }
        })
        .build(&window)?;
    Ok(Popup { window, view, parked: !is_menu })
}

fn main() -> wry::Result<()> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title("Cederic")
        .with_decorations(false)
        .with_undecorated_shadow(true)
        .with_inner_size(LogicalSize::new(1280.0, 820.0))
        .with_min_inner_size(LogicalSize::new(480.0, 320.0))
        .build(&event_loop)
        .expect("failed to create window");
    let dark = window.theme() == Theme::Dark;
    let frame: Rgba = if dark { (24, 25, 29, 255) } else { (223, 225, 230, 255) };
    window.set_background_color(Some(frame));

    let dir = data_dir();
    let history = HistoryStore::open(&dir);
    let mut context = WebContext::new(Some(dir.clone()));

    let p_chrome = proxy.clone();
    let size = window.inner_size().to_logical::<f64>(window.scale_factor());
    let chrome = WebViewBuilder::new_with_web_context(&mut context)
        .with_bounds(rect(0.0, 0.0, size.width, CHROME_HEIGHT))
        .with_html(CHROME_HTML)
        .with_background_color(frame)
        .with_additional_browser_args(BROWSER_ARGS)
        .with_scroll_bar_style(ScrollBarStyle::FluentOverlay)
        .with_devtools(cfg!(debug_assertions))
        .with_hotkeys_zoom(false)
        .with_ipc_handler(move |req| {
            if let Ok(cmd) = serde_json::from_str::<ChromeCmd>(req.body()) {
                let _ = p_chrome.send_event(UserEvent::Chrome(cmd));
            }
        })
        .build_as_child(&window)?;

    let menu = build_popup(&event_loop, &window, &mut context, proxy.clone(), true, dark)?;
    let suggest = build_popup(&event_loop, &window, &mut context, proxy.clone(), false, dark)?;

    let p_suggest = proxy.clone();
    let suggest_tx = suggest::spawn(move |res| {
        let _ = p_suggest.send_event(UserEvent::Suggestions(res));
    });
    // Warm up DNS + TLS to the suggestion server so the first keystroke is fast.
    let _ = suggest_tx.send(suggest::Request { seq: 0, query: String::new() });

    let mut browser = Browser {
        window,
        chrome,
        tabs: Vec::new(),
        spare: None,
        active: 0,
        next_id: 1,
        panel_height: 0.0,
        history,
        https_upgrades: 0,
        context,
        data_dir: dir,
        proxy,
        quit: false,
        dark,
        menu,
        menu_actions: Vec::new(),
        menu_shown: Instant::now(),
        suggest,
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

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(ev) => browser.handle(ev),
            Event::WindowEvent { window_id, event, .. } => browser.handle_window(window_id, event),
            _ => {}
        }
        if browser.quit {
            *control_flow = ControlFlow::Exit;
        }
    });
}
