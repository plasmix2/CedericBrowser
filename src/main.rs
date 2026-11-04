#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod crypto;
mod history;
mod input;

use history::{HistoryEntry, HistoryStore};
use serde::{Deserialize, Serialize};
use tao::{
    dpi::{LogicalPosition, LogicalSize},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    platform::windows::WindowBuilderExtWindows,
    window::{ResizeDirection, Window, WindowBuilder},
};
use wry::{
    BackgroundThrottlingPolicy, MemoryUsageLevel, NewWindowResponse, PageLoadEvent,
    PermissionResponse, Rect, ScrollBarStyle, WebContext, WebView, WebViewBuilder,
    WebViewBuilderExtWindows, WebViewExtWindows,
};

/// Base height of the tab strip + toolbar, in logical pixels.
const CHROME_HEIGHT: f64 = 84.0;

const CHROME_HTML: &str = include_str!("ui/chrome.html");
const NEWTAB_HTML: &str = include_str!("ui/newtab.html");

/// Every WebView sharing a data directory must use identical arguments.
/// Hardened for Wi-Fi search privacy:
/// - Built-in Async DNS + DNS-over-HTTPS (DoH via Cloudflare) so local Wi-Fi routers cannot see DNS queries.
/// - Encrypted Client Hello (ECH) + Post-Quantum Kyber TLS key exchange + HTTPS-First automatic upgrades.
/// - WebRTC non-proxied UDP disabled to prevent local Wi-Fi IP leaks.
/// - SmartScreen, DNS prefetch, hyperlink auditing (`<a ping>`), and background telemetry disabled.
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
    --enforce-webrtc-ip-permission-check";

/// Keyboard shortcuts and Wi-Fi form/beacon encryption guard for content pages.
/// Untrusted web content can only trigger the fixed `Shortcut` whitelist over IPC.
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
    if (e.key === 'F12') return 'devtools';
    return null;
  };
  window.addEventListener('keydown', (e) => {
    const cmd = map(e);
    if (cmd) { e.preventDefault(); e.stopPropagation(); window.ipc.postMessage(cmd); }
  }, true);

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

/// Finds the page's favicon; runs in the content page after it loads.
const FAVICON_SCRIPT: &str = r#"(() => {
  const l = document.querySelector('link[rel~="icon"], link[rel="shortcut icon"], link[rel="apple-touch-icon"]');
  return l ? l.href : (location.protocol.startsWith('http') ? location.origin + '/favicon.ico' : '');
})()"#;

type TabId = u32;

enum UserEvent {
    Chrome(ChromeCmd),
    Shortcut(Shortcut),
    NewTab(TabId, NewTabCmd),
    UpgradeHttps(TabId, String),
    Title(TabId, String),
    Load(TabId, PageLoadEvent, String),
    Favicon(TabId, String),
    OpenInNewTab(String),
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

struct Tab {
    id: TabId,
    view: WebView,
    title: String,
    url: String,
    favicon: String,
    loading: bool,
    is_newtab: bool,
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
    history: &'a [HistoryEntry],
    https_upgrades: u64,
}

#[derive(Serialize)]
struct NewTabState<'a> {
    history: &'a [HistoryEntry],
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
    proxy: EventLoopProxy<UserEvent>,
    quit: bool,
}

impl Browser {
    fn logical_size(&self) -> LogicalSize<f64> {
        self.window.inner_size().to_logical(self.window.scale_factor())
    }

    fn top_bar_height(&self) -> f64 {
        CHROME_HEIGHT + self.panel_height
    }

    fn chrome_bounds(&self) -> Rect {
        Rect {
            position: LogicalPosition::new(0.0, 0.0).into(),
            size: LogicalSize::new(self.logical_size().width, self.top_bar_height()).into(),
        }
    }

    fn content_bounds(&self) -> Rect {
        let size = self.logical_size();
        let top = self.top_bar_height();
        Rect {
            position: LogicalPosition::new(0.0, top).into(),
            size: LogicalSize::new(size.width, (size.height - top).max(0.0)).into(),
        }
    }

    /// Builds a hidden tab. It becomes visible when it is made the active tab.
    fn build_tab(&mut self, url: Option<&str>) -> wry::Result<Tab> {
        let id = self.next_id;
        self.next_id += 1;
        let bounds = self.content_bounds();

        let p_title = self.proxy.clone();
        let p_load = self.proxy.clone();
        let p_new = self.proxy.clone();
        let p_ipc = self.proxy.clone();
        let p_nav = self.proxy.clone();

        let builder = WebViewBuilder::new_with_web_context(&mut self.context)
            .with_bounds(bounds)
            .with_visible(false)
            .with_initialization_script(SHORTCUT_SCRIPT)
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
                }
            });
        let view = match url {
            Some(u) => builder.with_url(u),
            None => builder.with_html(NEWTAB_HTML),
        }
        .build_as_child(&self.window)?;

        Ok(Tab {
            id,
            view,
            title: if url.is_some() { String::new() } else { "New Tab".into() },
            url: url.unwrap_or_default().to_owned(),
            favicon: String::new(),
            loading: url.is_some(),
            is_newtab: url.is_none(),
        })
    }

    fn open_tab(&mut self, url: Option<String>) -> wry::Result<()> {
        let tab = match (&url, self.spare.take()) {
            (None, Some(spare)) => spare,
            (url, spare) => {
                self.spare = spare;
                self.build_tab(url.as_deref())?
            }
        };
        self.hide_active();
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        self.show_active();
        if url.is_none() {
            self.focus_address();
        }
        // Build the next spare after this tab is on screen, so it never delays the switch.
        let _ = self.proxy.send_event(UserEvent::Prewarm);
        Ok(())
    }

    fn close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        // Dropping the WebView tears down its renderer and frees its memory.
        drop(self.tabs.remove(idx));
        if self.tabs.is_empty() {
            let _ = self.open_tab(None);
            return;
        }
        if self.active >= self.tabs.len() || idx < self.active {
            self.active = self.active.saturating_sub(1).min(self.tabs.len() - 1);
        }
        self.show_active();
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
        let state = NewTabState {
            history: history.entries(),
        };
        if let Ok(json) = serde_json::to_string(&state) {
            let _ = view.evaluate_script(&format!("window.renderNewTab && window.renderNewTab({json})"));
        }
    }

    fn sync_ui(&self) {
        let Some(tab) = self.active_tab() else { return };
        let state = UiState {
            tabs: self
                .tabs
                .iter()
                .map(|t| TabState {
                    id: t.id,
                    title: &t.title,
                    favicon: &t.favicon,
                    loading: t.loading,
                })
                .collect(),
            active: tab.id,
            url: display_url(&tab.url),
            can_back: tab.view.can_go_back().unwrap_or(false),
            can_forward: tab.view.can_go_forward().unwrap_or(false),
            loading: tab.loading,
            maximized: self.window.is_maximized(),
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
        self.sync_ui();
    }

    fn navigate_active(&mut self, raw_input: &str) {
        let Some(resolved) = input::resolve_with_info(raw_input) else {
            return;
        };
        self.history
            .record(&resolved.query, &resolved.url, resolved.engine, resolved.is_search);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.is_newtab = false;
            let _ = tab.view.load_url(&resolved.url);
            let _ = tab.view.focus();
        }
        self.sync_ui();
    }

    fn handle_chrome(&mut self, cmd: ChromeCmd) {
        match cmd {
            ChromeCmd::Ready => self.sync_ui(),
            ChromeCmd::Navigate { input } => self.navigate_active(&input),
            ChromeCmd::NewTab => {
                let _ = self.open_tab(None);
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
            ChromeCmd::Back => self.with_active(|v| v.go_back()),
            ChromeCmd::Forward => self.with_active(|v| v.go_forward()),
            ChromeCmd::Reload => self.with_active(|v| v.reload()),
            ChromeCmd::Stop => self.with_active(|v| v.evaluate_script("window.stop()")),
            ChromeCmd::Devtools => {
                if let Some(t) = self.active_tab() {
                    t.view.open_devtools();
                }
            }
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
                    let _ = self.chrome.set_bounds(self.chrome_bounds());
                    let bounds = self.content_bounds();
                    if let Some(tab) = self.active_tab() {
                        let _ = tab.view.set_bounds(bounds);
                    }
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

    fn handle_shortcut(&mut self, s: Shortcut) {
        let n = self.tabs.len();
        match s {
            Shortcut::NewTab => self.handle_chrome(ChromeCmd::NewTab),
            Shortcut::CloseTab => self.close_tab(self.active),
            Shortcut::FocusAddress => self.focus_address(),
            Shortcut::History => {
                let _ = self.chrome.focus();
                let _ = self
                    .chrome
                    .evaluate_script("window.toggleHistoryDrawer && window.toggleHistoryDrawer()");
            }
            Shortcut::Reload => self.handle_chrome(ChromeCmd::Reload),
            Shortcut::NextTab => self.switch_to((self.active + 1) % n),
            Shortcut::PrevTab => self.switch_to((self.active + n - 1) % n),
            Shortcut::Back => self.handle_chrome(ChromeCmd::Back),
            Shortcut::Forward => self.handle_chrome(ChromeCmd::Forward),
            Shortcut::Devtools => self.handle_chrome(ChromeCmd::Devtools),
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
                let _ = self.open_tab(Some(target));
            }
            UserEvent::Prewarm => {
                if self.spare.is_none() {
                    self.spare = self.build_tab(None).ok();
                }
            }
            UserEvent::Title(id, title) => {
                if let Some(i) = self.index_of(id) {
                    self.tabs[i].title = title;
                    self.sync_ui();
                }
            }
            UserEvent::Favicon(id, icon) => {
                if let Some(i) = self.index_of(id) {
                    self.tabs[i].favicon = icon;
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
                if started && host_of(&url) != host_of(&tab.url) {
                    tab.favicon.clear();
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
}

/// The page reports its own favicon, so only accept web or inline image URLs.
fn parse_favicon(json: &str) -> Option<String> {
    let url: String = serde_json::from_str(json).ok()?;
    let ok = url.starts_with("https://") || url.starts_with("http://") || url.starts_with("data:image/");
    (ok && url.len() < 64 * 1024).then_some(url)
}

fn display_url(url: &str) -> &str {
    if url.starts_with("about:") || url.starts_with("data:") { "" } else { url }
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then_some(host)
}

fn data_dir() -> std::path::PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Cederic")
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

    let dir = data_dir();
    let history = HistoryStore::open(&dir);
    let mut context = WebContext::new(Some(dir));

    let p_chrome = proxy.clone();
    let size = window.inner_size().to_logical::<f64>(window.scale_factor());
    let chrome = WebViewBuilder::new_with_web_context(&mut context)
        .with_bounds(Rect {
            position: LogicalPosition::new(0.0, 0.0).into(),
            size: LogicalSize::new(size.width, CHROME_HEIGHT).into(),
        })
        .with_html(CHROME_HTML)
        .with_background_color((24, 25, 29, 255))
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
        proxy,
        quit: false,
    };
    let start = std::env::args().nth(1).and_then(|a| {
        let resolved = input::resolve_with_info(&a)?;
        browser
            .history
            .record(&resolved.query, &resolved.url, resolved.engine, resolved.is_search);
        Some(resolved.url)
    });
    browser.open_tab(start)?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(ev) => browser.handle(ev),
            Event::WindowEvent { event: WindowEvent::Resized(_), .. }
            | Event::WindowEvent { event: WindowEvent::ScaleFactorChanged { .. }, .. } => {
                browser.layout()
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => browser.quit = true,
            _ => {}
        }
        if browser.quit {
            *control_flow = ControlFlow::Exit;
        }
    });
}
