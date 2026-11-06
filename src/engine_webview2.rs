//! WebView2 backend for [`crate::engine`], through wry.
//!
//! This is the only file that knows about wry or Chromium flags. Every view, page or helper,
//! gets the same `BROWSER_ARGS`, scroll bar style, user agent and data folder.

use std::path::PathBuf;

use tao::{
    dpi::{LogicalPosition, LogicalSize},
    window::Window,
};
use wry::{
    BackgroundThrottlingPolicy, MemoryUsageLevel, NewWindowResponse, PageLoadEvent, PermissionKind,
    PermissionResponse, Rect, ScrollBarStyle, WebContext, WebView, WebViewBuilder,
    WebViewBuilderExtWindows, WebViewExtWindows,
};

use crate::engine::{
    Bounds, EngineError, LoadPhase, MemLevel, PageEngine, PageView, Perm, Result, Rgba, Role,
    Source, ViewConfig, ViewHandlers,
};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";

/// Every WebView sharing a data directory must use identical arguments (error `0x8007139F`
/// otherwise). `WryEngine::create_view` applies them to every view, so they cannot drift apart.
const BROWSER_ARGS: &str = "\
    --disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,AutofillServerCommunication,InterestFeedContentSuggestions,Translate,UserAgentClientHint \
    --enable-features=NetworkServiceInProcess2,AsyncDns,HappyEyeballsV3,EncryptedClientHello,BackForwardCache,ParallelDownloading,CanvasOopRasterization,UseEcoQoSForBackgroundProcess \
    --disable-blink-features=AutomationControlled \
    --autoplay-policy=no-user-gesture-required \
    --enable-quic \
    --ssl-version-min=tls1.2 \
    --disable-component-update \
    --enable-gpu-rasterization \
    --enable-zero-copy \
    --enable-accelerated-video-decode \
    --enable-smooth-scrolling \
    --enable-fast-unload \
    --no-pings \
    --disable-domain-reliability \
    --disable-client-side-phishing-detection \
    --disable-sync \
    --force-webrtc-ip-handling-policy=disable_non_proxied_udp \
    --enforce-webrtc-ip-permission-check \
    --remote-debugging-port=0";

impl From<wry::Error> for EngineError {
    fn from(e: wry::Error) -> Self {
        EngineError(e.to_string())
    }
}

fn rect(b: Bounds) -> Rect {
    Rect { position: LogicalPosition::new(b.x, b.y).into(), size: LogicalSize::new(b.w, b.h).into() }
}

fn perm_of(kind: PermissionKind) -> Perm {
    match kind {
        PermissionKind::Camera => Perm::Camera,
        PermissionKind::DisplayCapture => Perm::DisplayCapture,
        PermissionKind::Microphone => Perm::Microphone,
        PermissionKind::Notifications => Perm::Notifications,
        PermissionKind::Geolocation => Perm::Geolocation,
        _ => Perm::Other,
    }
}

/// The WebView2 engine. Owns the shared `WebContext` (data folder).
pub struct WryEngine {
    context: WebContext,
}

impl WryEngine {
    pub fn new(data_dir: PathBuf) -> Self {
        WryEngine { context: WebContext::new(Some(data_dir)) }
    }
}

impl PageEngine for WryEngine {
    fn create_view(
        &mut self,
        parent: &Window,
        config: ViewConfig<'_>,
        handlers: ViewHandlers,
    ) -> Result<Box<dyn PageView>> {
        let mut b = WebViewBuilder::new_with_web_context(&mut self.context)
            .with_bounds(rect(config.bounds))
            .with_visible(config.visible)
            .with_user_agent(USER_AGENT)
            .with_additional_browser_args(BROWSER_ARGS)
            .with_scroll_bar_style(ScrollBarStyle::FluentOverlay);
        for script in config.init_scripts {
            b = b.with_initialization_script(script);
        }
        if config.role == Role::Page {
            b = b
                .with_devtools(true)
                .with_hotkeys_zoom(true)
                .with_general_autofill_enabled(false)
                .with_background_throttling(BackgroundThrottlingPolicy::Disabled);
        }
        if let Some(bg) = config.background {
            b = b.with_background_color(bg);
        }
        if let Some(profile) = config.profile {
            b = b.with_profile_name(profile);
        }

        let ViewHandlers { on_navigate, on_title, on_load, on_new_window, on_message, on_permission } = handlers;
        if let Some(f) = on_permission {
            b = b.with_permission_handler(move |kind| {
                if f(perm_of(kind)) { PermissionResponse::Allow } else { PermissionResponse::Deny }
            });
        }
        if let Some(f) = on_navigate {
            b = b.with_navigation_handler(move |url| f(&url));
        }
        if let Some(f) = on_title {
            b = b.with_document_title_changed_handler(f);
        }
        if let Some(f) = on_load {
            b = b.with_on_page_load_handler(move |ev, url| {
                f(
                    match ev {
                        PageLoadEvent::Started => LoadPhase::Started,
                        PageLoadEvent::Finished => LoadPhase::Finished,
                    },
                    url,
                )
            });
        }
        if let Some(f) = on_new_window {
            b = b.with_new_window_req_handler(move |url, _features| {
                f(url);
                NewWindowResponse::Deny
            });
        }
        if let Some(f) = on_message {
            b = b.with_ipc_handler(move |req| f(req.body()));
        }
        b = match config.content {
            Source::Url(u) => b.with_url(u),
            Source::Html(h) => b.with_html(h),
        };
        Ok(Box::new(WryView { view: b.build_as_child(parent)? }))
    }
}

struct WryView {
    view: WebView,
}

impl PageView for WryView {
    fn load_url(&self, url: &str) -> Result<()> {
        Ok(self.view.load_url(url)?)
    }
    fn reload(&self) -> Result<()> {
        Ok(self.view.reload()?)
    }
    fn go_back(&self) -> Result<()> {
        Ok(self.view.go_back()?)
    }
    fn go_forward(&self) -> Result<()> {
        Ok(self.view.go_forward()?)
    }
    fn can_go_back(&self) -> Result<bool> {
        Ok(self.view.can_go_back()?)
    }
    fn can_go_forward(&self) -> Result<bool> {
        Ok(self.view.can_go_forward()?)
    }
    fn set_bounds(&self, bounds: Bounds) -> Result<()> {
        Ok(self.view.set_bounds(rect(bounds))?)
    }
    fn set_visible(&self, visible: bool) -> Result<()> {
        Ok(self.view.set_visible(visible)?)
    }
    fn set_memory_level(&self, level: MemLevel) -> Result<()> {
        Ok(self.view.set_memory_usage_level(match level {
            MemLevel::Normal => MemoryUsageLevel::Normal,
            MemLevel::Low => MemoryUsageLevel::Low,
        })?)
    }
    fn set_background(&self, color: Rgba) -> Result<()> {
        Ok(self.view.set_background_color(color)?)
    }
    fn focus(&self) -> Result<()> {
        Ok(self.view.focus()?)
    }
    fn focus_parent(&self) -> Result<()> {
        Ok(self.view.focus_parent()?)
    }
    fn eval(&self, js: &str) -> Result<()> {
        Ok(self.view.evaluate_script(js)?)
    }
    fn eval_with_result(&self, js: &str, callback: Box<dyn Fn(String) + Send>) -> Result<()> {
        Ok(self.view.evaluate_script_with_callback(js, callback)?)
    }
    fn clear_browsing_data(&self) -> Result<()> {
        Ok(self.view.clear_all_browsing_data()?)
    }
    fn open_devtools(&self) {
        self.view.open_devtools();
    }
}
