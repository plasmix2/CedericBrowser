#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod crypto;
mod devtools;
mod favicons;
mod anvil;
mod gfx;
mod history;
mod input;
mod pacer;
mod popups;
mod suggest;
mod tabler;
mod thumbs;
mod toolbar;
mod update;

use std::{
    collections::HashMap,
    num::NonZeroU32,
    path::PathBuf,
    rc::Rc,
    sync::mpsc::Sender,
    time::{Duration, Instant},
};

use gfx::{Brand, PersonaColor};
use history::{HistoryEntry, HistoryStore};
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
    window::{CursorIcon, Fullscreen, Icon as WinIcon, Theme, Window, WindowBuilder, WindowId},
};
use wry::{
    BackgroundThrottlingPolicy, MemoryUsageLevel, NewWindowResponse, PageLoadEvent,
    PermissionKind, PermissionResponse, Rect, ScrollBarStyle, WebContext, WebView,
    WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows,
};

/// Height of the native tab strip + toolbar, in logical pixels.
const CHROME_HEIGHT: f64 = toolbar::HEIGHT as f64;
/// Width of the draggable gap between the page and the DevTools pane.
const SPLITTER: f64 = 5.0;
const DEVTOOLS_MIN: f64 = 280.0;

const NEWTAB_HTML: &str = include_str!("ui/newtab.html");

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";

const STEALTH_SCRIPT: &str = r#"
(() => {
  try {
    delete Object.getPrototypeOf(navigator).webdriver;
  } catch (_) {}
  try {
    Object.defineProperty(navigator, 'webdriver', {
      get: () => undefined,
      configurable: true,
    });
  } catch (_) {}

  if (!window.chrome) window.chrome = {};
  if (window.chrome.webview) {
    const realWebview = window.chrome.webview;
    let internalCall = false;
    window.__cedericSendIpc = (msg) => {
      internalCall = true;
      try {
        if (window.ipc && typeof window.ipc.postMessage === 'function') {
          window.ipc.postMessage(msg);
        } else if (realWebview && typeof realWebview.postMessage === 'function') {
          realWebview.postMessage(msg);
        }
      } finally {
        internalCall = false;
      }
    };
    try {
      delete window.chrome.webview;
      Object.defineProperty(window.chrome, 'webview', {
        get() {
          if (internalCall) return realWebview;
          return undefined;
        },
        configurable: true,
        enumerable: false,
      });
    } catch (_) {}
  }
})();
"#;

/// Every WebView sharing a data directory must use identical arguments.
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

/// Keyboard shortcuts, the context menu, speculative link preconnect/prefetch, and Wi-Fi form
/// encryption guard for content pages. Untrusted web content can only trigger the fixed `Shortcut`
/// whitelist and a validated `ContextRequest` over IPC.
const SHORTCUT_SCRIPT: &str = r#"
(() => {
  if (window !== window.top) return;
  const post = (msg) => {
    if (typeof window.__cedericSendIpc === 'function') {
      window.__cedericSendIpc(msg);
    } else if (window.ipc && typeof window.ipc.postMessage === 'function') {
      window.ipc.postMessage(msg);
    }
  };
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
    const h = (location.hostname || '').toLowerCase();
    if (h.includes('youtube.com') || h.includes('youtu.be') || h.includes('googlevideo.com')) return;
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
    const h = (location.hostname || '').toLowerCase();
    if (h.includes('youtube.com') || h.includes('youtu.be')) return;
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
  const togglePip = async (preferCtx = false) => {
    try {
      if (document.pictureInPictureElement) {
        await document.exitPictureInPicture();
        post('pip_off');
        return true;
      }
      const v = bestVideo(preferCtx);
      if (!v) return false;
      v.disablePictureInPicture = false;
      v.removeAttribute('disablepictureinpicture');
      v.addEventListener('leavepictureinpicture', () => post('pip_off'), { once: true });
      await v.requestPictureInPicture();
      post('pip_on');
      return true;
    } catch (_) {
      return false;
    }
  };
  window.addEventListener('enterpictureinpicture', () => post('pip_on'), true);
  window.addEventListener('leavepictureinpicture', () => post('pip_off'), true);
  window.addEventListener('pagehide', () => {
    if (document.pictureInPictureElement) post('pip_off');
  }, true);

  const updateFs = () => {
    const isFs = !!(document.fullscreenElement || document.webkitFullscreenElement);
    post(isFs ? 'fs_on' : 'fs_off');
  };
  document.addEventListener('fullscreenchange', updateFs, true);
  document.addEventListener('webkitfullscreenchange', updateFs, true);

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
    if (e.key === 'F11') return 'fullscreen';
    if (e.key === 'F12' || (c && e.shiftKey && k === 'i')) return 'devtools';
    return null;
  };
  window.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') post('escape');
    const cmd = map(e);
    if (cmd === 'pip') {
      e.preventDefault(); e.stopPropagation();
      togglePip(false).then((ok) => { if (!ok) post('pip'); });
      return;
    }
    if (cmd) { e.preventDefault(); e.stopPropagation(); post(cmd); }
  }, true);
  // Popups never take focus, so a click or focus inside the page is what closes an open menu / address bar edit.
  window.addEventListener('pointerdown', () => post('dismiss'), true);
  window.addEventListener('focusin', () => post('dismiss'), true);
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
    ctxVideo = videoAt(e.clientX, e.clientY) || bestVideo(false);
    post(JSON.stringify({
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

  const sendBg = (rawUrl, keepRaw) => {
    const img = new Image();
    img.crossOrigin = 'anonymous';
    img.onload = () => {
      let data = rawUrl;
      if (!keepRaw) {
        try {
          const maxW = 1920, maxH = 1080;
          let w = img.naturalWidth || 1920, h = img.naturalHeight || 1080;
          const r = Math.min(1, maxW / w, maxH / h);
          w = Math.max(1, Math.round(w * r));
          h = Math.max(1, Math.round(h * r));
          const c = document.createElement('canvas');
          c.width = w; c.height = h;
          c.getContext('2d').drawImage(img, 0, 0, w, h);
          data = c.toDataURL('image/jpeg', 0.88);
        } catch (_) {}
      }
      let strip = '';
      try {
        const sc = document.createElement('canvas');
        sc.width = 640; sc.height = 84;
        const sx = sc.getContext('2d');
        const iw = img.naturalWidth || img.width || 640;
        const ih = img.naturalHeight || img.height || 360;
        const scale = Math.max(sc.width / iw, sc.height / ih);
        const dw = iw * scale, dh = ih * scale;
        sx.filter = 'blur(3px)';
        sx.drawImage(img, (sc.width - dw) / 2, (sc.height - dh) * 0.2, dw, dh);
        strip = sc.toDataURL('image/png');
      } catch (_) {}
      post(JSON.stringify({ nt: 'set_background', data, strip }));
    };
    img.onerror = () => {
      if (rawUrl) post(JSON.stringify({ nt: 'set_background', data: rawUrl, strip: '' }));
    };
    img.src = rawUrl;
  };
  const openBgPicker = () => {
    const existing = document.getElementById('bg-file');
    if (existing) { existing.click(); return; }
    const inp = document.createElement('input');
    inp.type = 'file';
    inp.accept = 'image/*';
    inp.style.display = 'none';
    inp.onchange = () => {
      const f = inp.files && inp.files[0];
      if (f && f.type.startsWith('image/')) {
        const reader = new FileReader();
        const keepGif = f.type === 'image/gif' && f.size <= 4 * 1024 * 1024;
        reader.onload = () => {
          if (typeof reader.result === 'string') sendBg(reader.result, keepGif);
        };
        reader.readAsDataURL(f);
      }
      inp.remove();
    };
    (document.body || document.documentElement).appendChild(inp);
    inp.click();
  };
  document.addEventListener('cederic:act', (e) => {
    const d = e.detail;
    if (!d) return;
    if (d.cmd === 'pip') togglePip(!!d.preferCtx);
    else if (d.cmd === 'pick_bg') openBgPicker();
    else if (d.cmd === 'apply_bg' && d.url) sendBg(d.url, false);
  });

  if (location.hostname === 'www.youtube.com' || location.hostname === 'm.youtube.com') {
    let lastPos = 0, lastRecover = 0;
    const recover = () => {
      if (lastPos < 0.5 || Date.now() - lastRecover < 4000) return;
      lastRecover = Date.now();
      const p = document.getElementById('movie_player');
      if (p && typeof p.loadVideoById === 'function' && typeof p.getVideoData === 'function') {
        const vd = p.getVideoData();
        if (vd && vd.video_id) {
          p.loadVideoById(vd.video_id, Math.max(0, lastPos - 0.5));
          if (typeof p.playVideo === 'function') setTimeout(() => p.playVideo(), 300);
          return;
        }
      }
      const u = new URL(location.href);
      if (u.pathname === '/watch' && u.searchParams.get('v')) {
        u.searchParams.set('t', Math.floor(Math.max(0, lastPos - 0.5)) + 's');
        location.replace(u.toString());
      }
    };
    setInterval(() => {
      const v = document.querySelector('video');
      if (v && v.currentTime > lastPos) lastPos = v.currentTime;
      const err = document.querySelector('.ytp-error, .ytp-error-content, [class*="ytp-error"]');
      const isVisible = err && (
        (typeof err.checkVisibility === 'function' && err.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true })) ||
        err.offsetParent !== null ||
        (err.getBoundingClientRect && err.getBoundingClientRect().height > 20)
      );
      if (isVisible) {
        recover();
      }
    }, 800);
    document.addEventListener('error', (e) => {
      if (e.target && e.target.tagName === 'VIDEO') {
        recover();
      }
    }, true);
  }
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
    brandName = window.__cedericBrandName || (b === 'uzaki' ? 'Uzaki' : 'Persona');
    const letter = (brandName.trim().charAt(0) || 'P').toUpperCase();
    const color = window.__cedericBrandColor || (b === 'uzaki' ? '#7366e6' : '#ff6a3d');
    const grad = `linear-gradient(90deg, ${color}, #ffffff)`;
    const badgeGrad = `linear-gradient(135deg, ${color}, ${color})`;
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
    FullscreenState(TabId, bool),
    /// Time to check which Google account is signed in (scheduled, never run from a page event).
    ProbeAccount,
    /// A newer version was downloaded and put in place; it runs after a restart.
    UpdateReady(String),
    /// The hidden account probe finished loading Google's account list.
    AccountProbeLoaded,
    /// What the probe read, for the profile it ran in.
    GoogleAccount(Option<String>, Option<GoogleAccount>),
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
    ToggleFullscreen,
    /// Clears cookies and all site data in every tab: signs out of every account.
    ClearSiteData,
    /// Tooltip centred under a toolbar button.
    Tip { text: String, x: f64, y: f64 },
    TipEnd,
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
    SelectPersona { id: String },
    CreatePersona {
        name: String,
        color: String,
        #[serde(default)]
        pfp: Option<String>,
    },
    UpdatePersona {
        id: String,
        name: String,
        color: String,
        #[serde(default)]
        pfp: Option<String>,
    },
    DeletePersona { id: String },
    SetPermissions {
        camera: bool,
        notifications: bool,
        microphone: bool,
        #[serde(default)]
        geolocation: bool,
    },
    SetBackground {
        #[serde(default)]
        data: String,
        #[serde(default)]
        strip: String,
    },
    AutoKey { key: String },
    /// The first-run setup screen was completed (or skipped).
    FinishSetup,
    /// The Settings button: open the native site-permissions dialog.
    OpenSettings,
    /// Setup: open Google's sign-in page for the active persona.
    LinkGoogle,
    /// Setup: look for the signed-in Google account again.
    CheckGoogle,
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
    Fullscreen,
    Escape,
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
            "fullscreen" => Self::Fullscreen,
            "escape" => Self::Escape,
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
    #[allow(dead_code)]
    SetBrand(Brand),
    SelectPersona(String),
    OpenNewPersona,
    OpenManagePersonas,
    ChooseBackground,
    SetBackgroundUrl(String),
    ClearBackground,
    ToggleFullscreen,
    ClearSiteData,
    OpenTab(String),
    RestartToUpdate,
    OpenSettings,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Bookmark {
    pub title: String,
    pub url: String,
}

/// The Google account signed in to the active persona's profile, as Google's own account list
/// reports it. Shown on the persona button and at the top of the persona menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoogleAccount {
    pub name: String,
    pub email: String,
    /// A `googleusercontent.com` photo URL, or empty.
    pub photo: String,
}

/// Where Google lists the accounts signed in to this browser profile (what Chrome's own avatar
/// button reads). It only answers a POST carrying the profile's cookies, so a hidden WebView
/// opens a tiny page on the same origin (`GOOGLE_PROBE_PAGE`) and posts from there.
const GOOGLE_ACCOUNTS_URL: &str = "https://accounts.google.com/ListAccounts?gpsia=1&source=ChromiumBrowser&json=standard";
const GOOGLE_PROBE_PAGE: &str = "https://accounts.google.com/robots.txt";
const GOOGLE_SIGN_IN_URL: &str = "https://accounts.google.com/ServiceLogin?continue=https%3A%2F%2Fmyaccount.google.com%2F";
const GOOGLE_ACCOUNT_URL: &str = "https://myaccount.google.com/";
const GOOGLE_SIGN_OUT_URL: &str = "https://accounts.google.com/Logout";

/// Parses the `ListAccounts` response: `["gaia.l.a.r", [["gaia.l.a", 1, name, email, photo, ...], ...]]`.
/// Untrusted page text: only well-formed, length-limited fields are kept, and only a
/// googleusercontent.com photo URL.
fn parse_google_accounts(text: &str) -> Option<GoogleAccount> {
    let start = text.find('[')?;
    let v: serde_json::Value = serde_json::from_str(&text[start..]).ok()?;
    for a in v.get(1)?.as_array()? {
        let field = |i: usize| a.get(i).and_then(|x| x.as_str()).unwrap_or("");
        // [9] is 1 while the session is valid; [14] is 1 for an account that was signed out.
        let valid = a.get(9).and_then(|x| x.as_i64()).unwrap_or(1) == 1;
        let signed_out = a.get(14).and_then(|x| x.as_i64()).unwrap_or(0) == 1;
        let email = field(3);
        if field(0) != "gaia.l.a" || !valid || signed_out || !email.contains('@') || email.len() > 254 {
            continue;
        }
        let name: String = field(2).chars().filter(|c| !c.is_control()).take(80).collect();
        let photo = field(4);
        let photo_ok = photo.len() < 2048
            && url::Url::parse(photo).ok().is_some_and(|u| {
                u.scheme() == "https" && u.host_str().is_some_and(|h| h == "googleusercontent.com" || h.ends_with(".googleusercontent.com"))
            });
        return Some(GoogleAccount {
            name: if name.trim().is_empty() { email.to_owned() } else { name },
            email: email.to_owned(),
            photo: if photo_ok { sized_google_photo(photo) } else { String::new() },
        });
    }
    None
}

/// Asks for a 96px square crop: googleusercontent photo URLs take a `=s<size>-c` suffix.
fn sized_google_photo(url: &str) -> String {
    let tail = url.rsplit('/').next().unwrap_or("");
    let base = match tail.rfind('=') {
        Some(i) => &url[..url.len() - (tail.len() - i)],
        None => url,
    };
    format!("{base}=s96-c")
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersonaProfile {
    pub id: String,
    pub name: String,
    pub color: String,
    #[serde(default)]
    pub pfp: Option<String>,
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
}

impl PersonaProfile {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new_cederic() -> Self {
        Self {
            id: "cederic".into(),
            name: "Cederic".into(),
            color: "orange".into(),
            pfp: None,
            bookmarks: Vec::new(),
        }
    }

    /// The single persona a fresh profile starts with. Its name is a placeholder: setup makes the user
    /// name it.
    pub fn new_main() -> Self {
        Self { id: "main".into(), name: "Persona".into(), color: "orange".into(), pfp: None, bookmarks: Vec::new() }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new_uzaki() -> Self {
        Self {
            id: "uzaki".into(),
            name: "Uzaki".into(),
            color: "slate".into(),
            pfp: None,
            bookmarks: Vec::new(),
        }
    }

    pub fn parsed_color(&self) -> PersonaColor {
        PersonaColor::from_str_name(&self.color)
    }

    /// The WebView2 profile holding this persona's cookies and sign-ins. The original persona keeps
    /// the default profile, so sign-ins made before personas had their own profiles stay with it.
    pub fn profile_name(&self) -> Option<String> {
        if self.id == "cederic" {
            return None;
        }
        let safe: String = self.id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').take(48).collect();
        Some(format!("persona_{safe}"))
    }

    pub fn badge_letter(&self) -> String {
        self.name
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "P".into())
    }
}

/// Drops the "Uzaki" and "Cederic" personas that used to be created for testing, when the user never
/// changed them. Returns true when the only persona left is an untouched "Cederic": the user has not
/// really set Persona up yet, so setup should run.
fn migrate_test_personas(personas: &mut Vec<PersonaProfile>) -> bool {
    if personas.len() > 1 {
        personas.retain(|p| !(p.id == "uzaki" && p.name == "Uzaki" && p.pfp.is_none() && p.bookmarks.is_empty()));
    }
    personas.len() == 1 && personas[0].id == "cederic" && personas[0].name == "Cederic" && personas[0].pfp.is_none()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SitePermissions {
    #[serde(default = "default_true")]
    pub camera: bool,
    #[serde(default = "default_true")]
    pub notifications: bool,
    #[serde(default = "default_true")]
    pub microphone: bool,
    #[serde(default)]
    pub geolocation: bool,
}

impl Default for SitePermissions {
    fn default() -> Self {
        Self {
            camera: true,
            notifications: true,
            microphone: true,
            geolocation: false,
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct PrefsData {
    #[serde(default)]
    active_persona: Option<String>,
    #[serde(default)]
    personas: Vec<PersonaProfile>,
    #[serde(default)]
    brand: Option<Brand>,
    #[serde(default)]
    bookmarks: Option<Vec<Bookmark>>,
    #[serde(default)]
    permissions: Option<SitePermissions>,
    /// Whether the first-run setup screen has been completed. Absent in prefs written before the
    /// setup screen existed; those users already have personas and are treated as set up.
    #[serde(default)]
    setup_done: Option<bool>,
}

struct Prefs {
    path: PathBuf,
    data_dir: PathBuf,
    active_persona_id: String,
    personas: Vec<PersonaProfile>,
    permissions: SitePermissions,
    setup_done: bool,
    /// The linked Google account's (name, email) for the setup screen; not saved.
    google_hint: Option<(String, String)>,
    bg_cache: HashMap<String, Option<String>>,
    strip_cache: HashMap<String, Option<Pixmap>>,
}

impl Prefs {
    fn open(data_dir: &std::path::Path) -> Self {
        let path = data_dir.join("prefs.json");
        let legacy_bg_path = data_dir.join("background.txt");
        let legacy_strip_path = data_dir.join("background_strip.png");
        let cederic_bg_path = data_dir.join("background_cederic.txt");
        let cederic_strip_path = data_dir.join("background_strip_cederic.png");

        if legacy_bg_path.exists() && !cederic_bg_path.exists() {
            let _ = std::fs::rename(&legacy_bg_path, &cederic_bg_path);
        }
        if legacy_strip_path.exists() && !cederic_strip_path.exists() {
            let _ = std::fs::rename(&legacy_strip_path, &cederic_strip_path);
        }

        let data: PrefsData = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();

        let mut setup_done = data.setup_done.unwrap_or(!data.personas.is_empty());
        let mut personas = data.personas;
        if personas.is_empty() {
            // The unit tests exercise persona switching and isolation, which needs two personas.
            personas = if cfg!(test) {
                let mut cederic = PersonaProfile::new_cederic();
                if let Some(bms) = data.bookmarks {
                    cederic.bookmarks = bms;
                }
                vec![cederic, PersonaProfile::new_uzaki()]
            } else {
                let mut main = PersonaProfile::new_main();
                if let Some(bms) = data.bookmarks {
                    main.bookmarks = bms;
                }
                vec![main]
            };
        }
        if !cfg!(test) && migrate_test_personas(&mut personas) {
            setup_done = false;
        }

        personas.truncate(5);

        let active_id = if let Some(active) = data.active_persona
            && personas.iter().any(|p| p.id == active)
        {
            active
        } else if let Some(brand) = data.brand {
            let legacy_id = match brand {
                Brand::Cederic => "cederic",
                Brand::Uzaki => "uzaki",
            };
            if personas.iter().any(|p| p.id == legacy_id) {
                legacy_id.to_string()
            } else {
                personas[0].id.clone()
            }
        } else {
            personas[0].id.clone()
        };

        let mut bg_cache = HashMap::new();
        let mut strip_cache = HashMap::new();

        // Backgrounds are data URLs that can be megabytes each, so only the active persona's is
        // read before the window appears; the others load when that persona is selected.
        if let Some(p) = personas.iter().find(|p| p.id == active_id) {
            let (bg, strip) = Self::read_background(data_dir, &p.id);
            bg_cache.insert(p.id.clone(), bg);
            strip_cache.insert(p.id.clone(), strip);
        }

        let permissions = data.permissions.unwrap_or_default();

        Self {
            path,
            data_dir: data_dir.to_path_buf(),
            active_persona_id: active_id,
            personas,
            permissions,
            setup_done,
            google_hint: None,
            bg_cache,
            strip_cache,
        }
    }

    fn read_background(data_dir: &std::path::Path, id: &str) -> (Option<String>, Option<Pixmap>) {
        let bg = std::fs::read_to_string(data_dir.join(format!("background_{id}.txt")))
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        let strip = std::fs::read(data_dir.join(format!("background_strip_{id}.png")))
            .ok()
            .and_then(|b| Pixmap::decode_png(&b).ok());
        (bg, strip)
    }

    /// Reads a persona's saved background from disk the first time it is needed.
    fn ensure_background(&mut self, id: &str) {
        if !self.bg_cache.contains_key(id) || !self.strip_cache.contains_key(id) {
            let (bg, strip) = Self::read_background(&self.data_dir, id);
            self.bg_cache.entry(id.to_string()).or_insert(bg);
            self.strip_cache.entry(id.to_string()).or_insert(strip);
        }
    }

    fn finish_setup(&mut self) {
        if !self.setup_done {
            self.setup_done = true;
            self.save();
        }
    }

    fn active_persona(&self) -> &PersonaProfile {
        self.personas
            .iter()
            .find(|p| p.id == self.active_persona_id)
            .unwrap_or(&self.personas[0])
    }

    fn active_persona_mut(&mut self) -> &mut PersonaProfile {
        let active = self.active_persona_id.clone();
        if let Some(pos) = self.personas.iter().position(|p| p.id == active) {
            &mut self.personas[pos]
        } else {
            &mut self.personas[0]
        }
    }

    fn active_id(&self) -> &str {
        &self.active_persona_id
    }

    fn active_name(&self) -> &str {
        &self.active_persona().name
    }

    fn active_color(&self) -> PersonaColor {
        self.active_persona().parsed_color()
    }

    fn active_badge_letter(&self) -> String {
        self.active_persona().badge_letter()
    }

    fn active_bookmarks(&self) -> &[Bookmark] {
        &self.active_persona().bookmarks
    }

    #[cfg(test)]
    fn bookmarks(&self) -> &[Bookmark] {
        self.active_bookmarks()
    }

    fn brand(&self) -> Brand {
        match self.active_color() {
            PersonaColor::Slate => Brand::Uzaki,
            _ => Brand::Cederic,
        }
    }

    #[cfg(test)]
    fn set_brand(&mut self, brand: Brand) {
        let target_id = match brand {
            Brand::Cederic => "cederic",
            Brand::Uzaki => "uzaki",
        };
        if self.personas.iter().any(|p| p.id == target_id) {
            self.set_active_persona(target_id);
        } else {
            let color = PersonaColor::from(brand);
            let active_id = self.active_persona_id.clone();
            let cur_name = self.active_name().to_string();
            self.update_persona(&active_id, &cur_name, color.name(), None);
        }
    }

    fn background(&self) -> Option<&str> {
        self.background_for(&self.active_persona_id)
    }

    fn background_for(&self, id_or_brand: impl AsRef<str>) -> Option<&str> {
        let key = id_or_brand.as_ref().to_ascii_lowercase();
        self.bg_cache.get(&key).and_then(|opt| opt.as_deref())
    }

    fn bg_strip(&self) -> Option<Pixmap> {
        self.bg_strip_for(&self.active_persona_id)
    }

    fn bg_strip_for(&self, id_or_brand: impl AsRef<str>) -> Option<Pixmap> {
        let key = id_or_brand.as_ref().to_ascii_lowercase();
        self.strip_cache.get(&key).and_then(|opt| opt.clone())
    }

    fn bg_path(&self, id: &str) -> PathBuf {
        self.data_dir.join(format!("background_{id}.txt"))
    }

    fn strip_path(&self, id: &str) -> PathBuf {
        self.data_dir.join(format!("background_strip_{id}.png"))
    }

    fn save(&self) {
        let data = PrefsData {
            active_persona: Some(self.active_persona_id.clone()),
            personas: self.personas.clone(),
            brand: Some(self.brand()),
            bookmarks: Some(self.active_bookmarks().to_vec()),
            permissions: Some(self.permissions.clone()),
            setup_done: Some(self.setup_done),
        };
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

    pub fn set_permissions(&mut self, camera: bool, notifications: bool, microphone: bool, geolocation: bool) {
        self.permissions.camera = camera;
        self.permissions.notifications = notifications;
        self.permissions.microphone = microphone;
        self.permissions.geolocation = geolocation;
        self.save();
    }

    fn set_active_persona(&mut self, id: &str) -> bool {
        if self.personas.iter().any(|p| p.id == id) {
            self.active_persona_id = id.to_string();
            self.ensure_background(id);
            self.save();
            true
        } else {
            false
        }
    }

    fn create_persona(
        &mut self,
        name: &str,
        color: &str,
        pfp: Option<String>,
    ) -> Result<String, &'static str> {
        if self.personas.len() >= 5 {
            return Err("Maximum 5 personas allowed");
        }
        let clean_name = name.trim();
        let final_name = if clean_name.is_empty() {
            format!("Persona {}", self.personas.len() + 1)
        } else {
            clean_name.to_string()
        };
        let valid_color = PersonaColor::from_str_name(color).name().to_string();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        // Two personas created within the same millisecond would otherwise share an id, and
        // deleting or editing one would hit the other.
        let mut id = format!("p_{stamp}");
        let mut n = 1;
        while self.personas.iter().any(|p| p.id == id) {
            n += 1;
            id = format!("p_{stamp}_{n}");
        }
        let persona = PersonaProfile {
            id: id.clone(),
            name: final_name,
            color: valid_color,
            pfp: pfp.filter(|s| s.starts_with("data:image/") || s.starts_with("http")),
            bookmarks: Vec::new(),
        };
        self.personas.push(persona);
        self.active_persona_id = id.clone();
        self.save();
        Ok(id)
    }

    fn update_persona(
        &mut self,
        id: &str,
        name: &str,
        color: &str,
        pfp: Option<Option<String>>,
    ) -> bool {
        if let Some(p) = self.personas.iter_mut().find(|p| p.id == id) {
            let clean_name = name.trim();
            if !clean_name.is_empty() {
                p.name = clean_name.to_string();
            }
            p.color = PersonaColor::from_str_name(color).name().to_string();
            if let Some(new_pfp) = pfp {
                p.pfp = new_pfp.filter(|s| s.starts_with("data:image/") || s.starts_with("http"));
            }
            self.save();
            true
        } else {
            false
        }
    }

    fn delete_persona(&mut self, id: &str) -> Result<(), &'static str> {
        if self.personas.len() <= 1 {
            return Err("Cannot delete the last persona");
        }
        if let Some(pos) = self.personas.iter().position(|p| p.id == id) {
            self.personas.remove(pos);
            if self.active_persona_id == id {
                self.active_persona_id = self.personas[0].id.clone();
                let first = self.active_persona_id.clone();
                self.ensure_background(&first);
            }
            let bg_p = self.bg_path(id);
            let strip_p = self.strip_path(id);
            let _ = std::fs::remove_file(bg_p);
            let _ = std::fs::remove_file(strip_p);
            self.bg_cache.remove(id);
            self.strip_cache.remove(id);
            self.save();
            Ok(())
        } else {
            Err("Persona not found")
        }
    }

    fn set_background(
        &mut self,
        id_or_brand: impl AsRef<str>,
        data: Option<String>,
        strip_png: Option<&str>,
    ) {
        let id = id_or_brand.as_ref().to_ascii_lowercase();
        let bg_path = self.bg_path(&id);
        let strip_path = self.strip_path(&id);
        let clean = data
            .map(|s| s.trim().to_owned())
            .filter(|s| s.starts_with("data:image/") || s.starts_with("https://") || s.starts_with("http://"));

        if let Some(bg) = clean {
            if let Some(p) = bg_path.parent() {
                let _ = std::fs::create_dir_all(p);
            }
            let _ = std::fs::write(&bg_path, &bg);
            let strip_pm = if let Some(b64) = strip_png.map(str::trim).filter(|s| !s.is_empty()) {
                let raw = b64.strip_prefix("data:image/png;base64,").unwrap_or(b64);
                if let Some(bytes) = favicons::base64_decode(raw)
                    && let Ok(pm) = Pixmap::decode_png(&bytes)
                {
                    let _ = std::fs::write(&strip_path, &bytes);
                    Some(pm)
                } else {
                    None
                }
            } else {
                let _ = std::fs::remove_file(&strip_path);
                None
            };
            self.bg_cache.insert(id.clone(), Some(bg));
            self.strip_cache.insert(id, strip_pm);
        } else {
            let _ = std::fs::remove_file(&bg_path);
            let _ = std::fs::remove_file(&strip_path);
            self.bg_cache.insert(id.clone(), None);
            self.strip_cache.insert(id, None);
        }
    }

    fn is_bookmarked(&self, url: &str) -> bool {
        !url.is_empty() && self.active_bookmarks().iter().any(|b| b.url == url)
    }

    fn toggle_bookmark(&mut self, title: &str, url: &str) -> bool {
        if url.is_empty() || url.starts_with("about:") || url.starts_with("data:") {
            return false;
        }
        let bms = &mut self.active_persona_mut().bookmarks;
        if let Some(pos) = bms.iter().position(|b| b.url == url) {
            bms.remove(pos);
            self.save();
            false
        } else {
            let clean = title.trim();
            let t = if clean.is_empty() || clean == "New Tab" {
                host_of(url).unwrap_or(url).to_owned()
            } else {
                clean.chars().take(60).collect()
            };
            bms.insert(0, Bookmark { title: t, url: url.to_owned() });
            if bms.len() > 100 {
                bms.truncate(100);
            }
            self.save();
            true
        }
    }

    fn remove_bookmark(&mut self, url: &str) -> bool {
        let bms = &mut self.active_persona_mut().bookmarks;
        let before = bms.len();
        bms.retain(|b| b.url != url);
        if bms.len() != before {
            self.save();
            true
        } else {
            false
        }
    }

    #[allow(dead_code)]
    fn delete_bookmark(&mut self, url: &str) -> bool {
        self.remove_bookmark(url)
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

static LOGO_DATA_URI: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!("data:image/png;base64,{}", base64_encode(include_bytes!("ui/logo_64.png")))
});

#[derive(Serialize)]
struct PersonaDto<'a> {
    id: &'a str,
    name: &'a str,
    color: &'a str,
    color_hex: &'a str,
    pfp: Option<&'a str>,
    is_active: bool,
    bookmark_count: usize,
}

#[derive(Serialize)]
struct NewTabState<'a> {
    brand: Brand,
    personas: Vec<PersonaDto<'a>>,
    active_persona_id: &'a str,
    persona_name: &'a str,
    persona_color: &'a str,
    persona_color_hex: &'a str,
    persona_pfp: Option<&'a str>,
    bookmarks: &'a [Bookmark],
    history: &'a [HistoryEntry],
    background: Option<&'a str>,
    logo: &'a str,
    can_create: bool,
    can_delete: bool,
    allow_camera: bool,
    allow_notifications: bool,
    allow_microphone: bool,
    allow_geolocation: bool,
    setup_done: bool,
    google: Option<(&'a str, &'a str)>,
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
    fullscreen: bool,
    html_fullscreen: bool,
    last_title: String,
    /// The Google account signed in to the active persona's profile, once the probe has read it.
    google: Option<GoogleAccount>,
    /// Hidden WebView reading Google's account list (see `probe_google_account`), while it runs.
    account_probe: Option<(Option<String>, WebView, Instant)>,
    /// A probe was asked for while one was running: run another when it ends.
    probe_again: bool,
    /// A probe is already scheduled to run shortly.
    probe_scheduled: bool,
    /// A downloaded update waiting for a restart: its version.
    update_ready: Option<String>,
    /// The exe this process started from (an update replaces the file at this path).
    exe_path: Option<PathBuf>,
    allow_camera: std::sync::Arc<std::sync::atomic::AtomicBool>,
    allow_notifications: std::sync::Arc<std::sync::atomic::AtomicBool>,
    allow_microphone: std::sync::Arc<std::sync::atomic::AtomicBool>,
    allow_geolocation: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Browser {
    fn logical_size(&self) -> LogicalSize<f64> {
        self.window.inner_size().to_logical(self.window.scale_factor())
    }

    fn top_bar_height(&self) -> f64 {
        if self.fullscreen {
            0.0
        } else {
            CHROME_HEIGHT
        }
    }

    fn devtools_width(&self) -> f64 {
        if self.fullscreen || !self.devtools.open || self.devtools.view.is_none() {
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
        let (strip, toolbar, _) = self.prefs.active_color().surface_tints(self.dark);
        if self.dark {
            (strip.0, strip.1, strip.2, 255)
        } else {
            (toolbar.0, toolbar.1, toolbar.2, 255)
        }
    }

    /// Builds a hidden tab. It becomes visible when it is made the active tab.
    fn build_tab(&mut self, url: Option<&str>) -> wry::Result<Tab> {
        let id = self.next_id;
        self.next_id += 1;
        let bounds = self.content_bounds();
        let bg = self.newtab_color();
        let brand_key = if self.prefs.active_color() == PersonaColor::Slate { "uzaki" } else { "cederic" };
        let persona_name = self.prefs.active_name().to_string();
        let persona_color_hex = self.prefs.active_color().hex_code();

        let p_title = self.proxy.clone();
        let p_load = self.proxy.clone();
        let p_new = self.proxy.clone();
        let p_ipc = self.proxy.clone();
        let p_nav = self.proxy.clone();
        let cam = self.allow_camera.clone();
        let notif = self.allow_notifications.clone();
        let mic = self.allow_microphone.clone();
        let geo = self.allow_geolocation.clone();

        let mut builder = WebViewBuilder::new_with_web_context(&mut self.context)
            .with_bounds(bounds)
            .with_visible(false)
            .with_user_agent(USER_AGENT)
            .with_initialization_script(STEALTH_SCRIPT)
            .with_initialization_script(format!(
                "if (window === window.top && location.hostname === 'search.brave.com') {{ window.__cedericBrand = {brand_key:?}; window.__cedericBrandName = {persona_name:?}; window.__cedericBrandColor = {persona_color_hex:?}; }}"
            ))
            .with_initialization_script(SHORTCUT_SCRIPT)
            .with_initialization_script(BRAND_SCRIPT)
            .with_additional_browser_args(BROWSER_ARGS)
            .with_scroll_bar_style(ScrollBarStyle::FluentOverlay)
            .with_devtools(true)
            .with_hotkeys_zoom(true)
            .with_general_autofill_enabled(false)
            .with_background_throttling(BackgroundThrottlingPolicy::Disabled)
            .with_permission_handler(move |kind| match kind {
                PermissionKind::Camera | PermissionKind::DisplayCapture => {
                    if cam.load(std::sync::atomic::Ordering::Relaxed) {
                        PermissionResponse::Allow
                    } else {
                        PermissionResponse::Deny
                    }
                }
                PermissionKind::Notifications => {
                    if notif.load(std::sync::atomic::Ordering::Relaxed) {
                        PermissionResponse::Allow
                    } else {
                        PermissionResponse::Deny
                    }
                }
                PermissionKind::Microphone => {
                    if mic.load(std::sync::atomic::Ordering::Relaxed) {
                        PermissionResponse::Allow
                    } else {
                        PermissionResponse::Deny
                    }
                }
                PermissionKind::Geolocation => {
                    if geo.load(std::sync::atomic::Ordering::Relaxed) {
                        PermissionResponse::Allow
                    } else {
                        PermissionResponse::Deny
                    }
                }
                _ => PermissionResponse::Allow,
            })
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
                } else if body == "fs_on" {
                    let _ = p_ipc.send_event(UserEvent::FullscreenState(id, true));
                } else if body == "fs_off" {
                    let _ = p_ipc.send_event(UserEvent::FullscreenState(id, false));
                } else if let Some(s) = Shortcut::parse(body) {
                    let _ = p_ipc.send_event(UserEvent::Shortcut(s));
                } else if let Ok(cmd) = serde_json::from_str::<NewTabCmd>(body) {
                    let _ = p_ipc.send_event(UserEvent::NewTab(id, cmd));
                } else if let Some(r) = ContextRequest::parse(body) {
                    let _ = p_ipc.send_event(UserEvent::PageMenu(id, r));
                }
            });
        // Each persona has its own profile: separate cookies, so separate Google sign-ins.
        if let Some(profile) = self.prefs.active_persona().profile_name() {
            builder = builder.with_profile_name(profile);
        }
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
        if idx == self.active && self.html_fullscreen {
            self.set_fullscreen(false, true);
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
            if self.html_fullscreen {
                self.set_fullscreen(false, true);
            }
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
            if t.pip || t.url.contains("youtube.com") || t.url.contains("youtu.be") {
                // Keep a tab with active Picture-in-Picture or YouTube playback alive while
                // parked outside the visible client area, so switching tabs never drops video/audio.
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
                Self::push_newtab_state(&tab.view, &self.prefs, &self.history);
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

    fn push_newtab_state(view: &WebView, prefs: &Prefs, history: &HistoryStore) {
        let active = prefs.active_persona();
        let color = active.parsed_color();
        let personas = prefs
            .personas
            .iter()
            .map(|p| PersonaDto {
                id: &p.id,
                name: &p.name,
                color: &p.color,
                color_hex: p.parsed_color().hex_code(),
                pfp: p.pfp.as_deref(),
                is_active: p.id == prefs.active_persona_id,
                bookmark_count: p.bookmarks.len(),
            })
            .collect();
        let state = NewTabState {
            brand: prefs.brand(),
            personas,
            active_persona_id: &prefs.active_persona_id,
            persona_name: &active.name,
            persona_color: color.name(),
            persona_color_hex: color.hex_code(),
            persona_pfp: active.pfp.as_deref(),
            bookmarks: prefs.active_bookmarks(),
            history: history.entries(),
            background: prefs.background(),
            logo: LOGO_DATA_URI.as_str(),
            can_create: prefs.personas.len() < 5,
            can_delete: prefs.personas.len() > 1,
            allow_camera: prefs.permissions.camera,
            allow_notifications: prefs.permissions.notifications,
            allow_microphone: prefs.permissions.microphone,
            allow_geolocation: prefs.permissions.geolocation,
            setup_done: prefs.setup_done,
            google: prefs.google_hint.as_ref().map(|(n, e)| (n.as_str(), e.as_str())),
        };
        if let Ok(json) = serde_json::to_string(&state) {
            let _ = view.evaluate_script(&format!("window.renderNewTab && window.renderNewTab({json})"));
        }
    }

    fn sync_ui(&mut self) {
        let Some(tab) = self.active_tab() else { return };
        let brand = self.prefs.brand();
        let color = self.prefs.active_color();
        let badge_letter = self.prefs.active_badge_letter();
        let active_name = self.prefs.active_name().to_string();
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
            url: display_url(&tab.url, &active_name),
            can_back: tab.can_back,
            can_forward: tab.can_forward,
            loading: tab.loading,
            maximized: self.window.is_maximized(),
            devtools: self.devtools.open,
            pip: self.tabs.iter().any(|t| t.pip),
            bookmarked: self.prefs.is_bookmarked(&tab.url),
            brand,
            color,
            badge_letter,
            update_ready: self.update_ready.is_some(),
            pfp: match &self.google {
                Some(g) if !g.photo.is_empty() => g.photo.clone(),
                _ => self.prefs.active_persona().pfp.clone().unwrap_or_default(),
            },
        };
        if tab.is_newtab {
            Self::push_newtab_state(&tab.view, &self.prefs, &self.history);
        }
        let title = if tab.title.is_empty() { &active_name } else { &tab.title };
        // Pre-release builds name themselves (PERSONA_BUILD=<git hash> at compile time), so the
        // running exe can be told apart from the taskbar or Alt+Tab.
        let full_title = match option_env!("PERSONA_BUILD") {
            Some(build) => format!("{title} - {active_name} [{build}]"),
            None => format!("{title} - {active_name}"),
        };
        if self.last_title != full_title {
            self.window.set_title(&full_title);
            self.last_title = full_title;
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
        let Some(resolved) = input::resolve_with_info(&unmask(raw_input)) else {
            return;
        };
        // Start the load before anything else (closing the dropdown, history, repainting) so the
        // network request leaves as early as possible.
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.is_newtab = false;
            let _ = tab.view.load_url(&resolved.url);
            let _ = tab.view.focus();
        }
        self.close_suggest();
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
        self.hide_tip();
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
            && !cur.url.contains("youtube.com")
            && !cur.url.contains("youtu.be")
        {
            let _ = self.thumbs_tx.send(thumbs::Request::Capture(cur.id));
        }
        let shown = display_url(&tab.url, self.prefs.active_name());
        let card = Card {
            title: if tab.title.is_empty() { "New Tab".into() } else { tab.title.clone() },
            host: host_of(&shown).unwrap_or_default().to_owned(),
            img: if active { None } else { tab.thumb.clone() },
        };
        let x = x.clamp(0.0, (self.logical_size().width - w).max(0.0));
        let pos = self.to_screen(x, y);
        self.popups.show(PopupKind::Preview, Content::Preview(card), pos, w as f32);
    }

    /// Tooltip centred under a toolbar button, in its own native popup window.
    fn show_tip(&mut self, text: String, cx: f64, y: f64) {
        if self.popups.is_open(PopupKind::Menu) {
            return;
        }
        let w = self.popups.tip_width(&text) as f64;
        let x = (cx - w / 2.0).clamp(4.0, (self.logical_size().width - w - 4.0).max(4.0));
        let pos = self.to_screen(x, y);
        self.popups.show(PopupKind::Tip, Content::Tip(text), pos, w as f32);
    }

    fn hide_tip(&mut self) {
        self.popups.hide(PopupKind::Tip);
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
        items.push(MenuItem::sep());
        items.push(MenuItem::new("Clear cookies and site data", MenuAction::ClearSiteData).hint("Signs you out"));
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
                Self::push_newtab_state(&tab.view, &self.prefs, &self.history);
            }
        }
        if let Some(spare) = &self.spare
            && spare.is_newtab
        {
            Self::push_newtab_state(&spare.view, &self.prefs, &self.history);
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

        if self.prefs.active_bookmarks().is_empty() {
            items.push(MenuItem::new("No saved bookmarks yet", MenuAction::ToggleBookmark).enabled(false));
        } else {
            for b in self.prefs.active_bookmarks().iter().take(MAX) {
                let shown = display_url(&b.url, self.prefs.active_name());
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

    /// Theme & persona dropdown under the palette button: switch between personas, create new,
    /// or choose / clear a custom background image.
    fn theme_menu(&mut self, right: f64, y: f64) {
        let active_id = self.prefs.active_id().to_string();
        let mut items = Vec::new();
        if let Some(v) = &self.update_ready {
            items.push(MenuItem::new(format!("Restart to update to Persona {v}"), MenuAction::RestartToUpdate).hint("Update"));
            items.push(MenuItem::sep());
        }
        match &self.google {
            Some(g) => {
                items.push(MenuItem::new(g.name.clone(), MenuAction::OpenTab(GOOGLE_ACCOUNT_URL.into())).hint("Google").enabled(false));
                if g.email != g.name {
                    items.push(MenuItem::new(g.email.clone(), MenuAction::OpenTab(GOOGLE_ACCOUNT_URL.into())).enabled(false));
                }
                items.push(MenuItem::new("Manage your Google Account", MenuAction::OpenTab(GOOGLE_ACCOUNT_URL.into())));
                items.push(MenuItem::new("Sign out of Google", MenuAction::OpenTab(GOOGLE_SIGN_OUT_URL.into())));
            }
            None => items.push(MenuItem::new("Sign in to Google", MenuAction::OpenTab(GOOGLE_SIGN_IN_URL.into()))),
        }
        items.push(MenuItem::sep());
        for p in &self.prefs.personas {
            let is_cur = p.id == active_id;
            let mark = if is_cur { "✓  " } else { "    " };
            let label = format!("{mark}{}", p.name);
            items.push(MenuItem::new(label, MenuAction::SelectPersona(p.id.clone())));
        }
        if self.prefs.personas.len() < 5 {
            items.push(MenuItem::new("New persona…", MenuAction::OpenNewPersona));
        }
        items.push(MenuItem::new("Manage personas…", MenuAction::OpenManagePersonas));
        items.push(MenuItem::sep());
        let has_bg = self.prefs.background().is_some();
        items.push(MenuItem::new(if has_bg { "Change background…" } else { "Set background…" }, MenuAction::ChooseBackground));
        if has_bg {
            items.push(MenuItem::new("Remove background", MenuAction::ClearBackground));
        }
        items.push(MenuItem::new("Site permissions…", MenuAction::OpenSettings));
        self.show_menu(items, (right - 260.0).max(0.0), y);
    }

    fn set_permissions(&mut self, camera: bool, notifications: bool, microphone: bool, geolocation: bool) {
        self.allow_camera.store(camera, std::sync::atomic::Ordering::Relaxed);
        self.allow_notifications.store(notifications, std::sync::atomic::Ordering::Relaxed);
        self.allow_microphone.store(microphone, std::sync::atomic::Ordering::Relaxed);
        self.allow_geolocation.store(geolocation, std::sync::atomic::Ordering::Relaxed);
        self.prefs.set_permissions(camera, notifications, microphone, geolocation);
        self.sync_all_newtabs();
    }

    /// The site-permissions dialog, drawn by the ANVL engine in its own popup window.
    fn show_settings(&mut self) {
        const W: f32 = 420.0;
        let item = |label: &str, desc: &str, on: bool| popups::ToggleEntry { label: label.into(), desc: desc.into(), on };
        let perms = &self.prefs.permissions;
        let content = Content::Modal {
            title: "Site permissions".into(),
            note: "On: sites may use it without asking again. Off: the request is blocked.".into(),
            items: vec![
                item("Camera & video", "Websites may use your webcam", perms.camera),
                item("Microphone & audio", "Websites may use your microphone", perms.microphone),
                item("Notifications", "Sites may send desktop notifications", perms.notifications),
                item("Location", "Websites may read your location", perms.geolocation),
            ],
        };
        self.close_menu();
        self.hide_preview();
        self.hide_tip();
        let first_open = !self.popups.is_open(PopupKind::Scrim);
        self.show_scrim(first_open);
        // Centred over the window, a little above the middle.
        let size = self.logical_size();
        let h = 74.0 + 4.0 * 56.0 + 62.0;
        let pos = self.to_screen(((size.width - W as f64) / 2.0).max(0.0), ((size.height - h as f64) / 2.0 - 20.0).max(0.0));
        self.popups.show(PopupKind::Modal, content, pos, W);
    }

    /// Covers the page with a blurred, dimmed snapshot of it while a dialog is open. The snapshot
    /// is the active tab's last thumbnail (and a fresh one is requested); until one exists the page
    /// is only dimmed.
    fn show_scrim(&mut self, request_fresh: bool) {
        let size = self.logical_size();
        let top = self.top_bar_height();
        let img = self.tabs.get(self.active).and_then(|t| t.thumb.as_ref()).map(|t| anvil::blur_pixmap(t, 6));
        if request_fresh && let Some(t) = self.tabs.get(self.active) {
            let _ = self.thumbs_tx.send(thumbs::Request::Capture(t.id));
        }
        let pos = self.to_screen(0.0, top);
        let h = (size.height - top).max(1.0) as f32;
        self.popups.show(PopupKind::Scrim, Content::Scrim { img, h }, pos, size.width as f32);
    }

    fn hide_modal(&mut self) {
        self.popups.hide(PopupKind::Modal);
        self.popups.hide(PopupKind::Scrim);
    }

    fn open_settings(&mut self) {
        self.show_settings();
    }

    fn apply_background(&mut self, data: Option<String>, strip_png: Option<&str>) {
        let active_id = self.prefs.active_id().to_string();
        self.prefs.set_background(&active_id, data, strip_png);
        self.toolbar.set_background(self.prefs.bg_strip());
        self.sync_all_newtabs();
        self.sync_ui();
    }

    fn choose_background(&mut self) {
        let js = "document.dispatchEvent(new CustomEvent('cederic:act', { detail: { cmd: 'pick_bg' } }))";
        self.with_active(|v| v.evaluate_script(js));
    }

    fn open_persona_editor(&mut self, id: Option<&str>) {
        let script = match id {
            Some(i) => format!("window.__personaOpenEditor && window.__personaOpenEditor({i:?});"),
            None => "window.__personaOpenEditor && window.__personaOpenEditor(null);".to_string(),
        };
        let is_newtab = self.tabs.get(self.active).is_some_and(|t| t.is_newtab);
        if is_newtab {
            self.with_active(|v| v.evaluate_script(&script));
        } else {
            let _ = self.open_tab(None, None, true);
            self.with_active(|v| v.evaluate_script(&script));
        }
    }

    /// Reads which Google account is signed in to the active persona's profile. Loads Google's
    /// account list in a hidden, throwaway WebView of that profile and reads the page text back
    /// with a script callback, so the page itself gets no channel to the browser.
    fn probe_google_account(&mut self) {
        // A probe that never finished (offline, blocked) must not block every later one.
        if self.account_probe.as_ref().is_some_and(|(_, _, t)| t.elapsed() < Duration::from_secs(20)) {
            self.probe_again = true;
            return;
        }
        self.account_probe = None;
        let profile = self.prefs.active_persona().profile_name();
        let proxy = self.proxy.clone();
        let mut builder = WebViewBuilder::new_with_web_context(&mut self.context)
            .with_bounds(rect(-10.0, -10.0, 1.0, 1.0))
            .with_visible(false)
            .with_user_agent(USER_AGENT)
            // Same arguments and scroll bar style as every other WebView (see BROWSER_ARGS).
            .with_additional_browser_args(BROWSER_ARGS)
            .with_scroll_bar_style(ScrollBarStyle::FluentOverlay)
            .with_navigation_handler(|u| u == GOOGLE_PROBE_PAGE)
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .with_on_page_load_handler(move |ev, url| {
                if matches!(ev, PageLoadEvent::Finished) && url == GOOGLE_PROBE_PAGE {
                    let _ = proxy.send_event(UserEvent::AccountProbeLoaded);
                }
            })
            .with_url(GOOGLE_PROBE_PAGE);
        if let Some(name) = &profile {
            builder = builder.with_profile_name(name.clone());
        }
        if let Ok(view) = builder.build_as_child(&self.window) {
            self.account_probe = Some((profile, view, Instant::now()));
        }
    }

    /// Checks the Google account a moment from now. Never builds the probe WebView from inside a
    /// page event: creating a WebView runs a nested message loop, and during sign-in Windows may be
    /// showing its own modal dialog (Windows Hello for passkeys), which must not be re-entered.
    fn schedule_account_probe(&mut self, delay: Duration) {
        if std::mem::replace(&mut self.probe_scheduled, true) {
            return;
        }
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            let _ = proxy.send_event(UserEvent::ProbeAccount);
        });
    }

    fn account_probe_loaded(&mut self) {
        let Some((profile, view, _)) = &self.account_probe else { return };
        let proxy = self.proxy.clone();
        let profile = profile.clone();
        // Synchronous so the script returns the text itself (an async result would not come back).
        let read_js = format!(
            "(() => {{ try {{ const x = new XMLHttpRequest(); x.open('POST', {GOOGLE_ACCOUNTS_URL:?}, false); x.send(' '); return x.status === 200 ? x.responseText : ''; }} catch (_) {{ return ''; }} }})()"
        );
        let read = view.evaluate_script_with_callback(&read_js, move |json| {
            // The callback gets the script's result as JSON: here, a JSON string.
            let text: String = serde_json::from_str(&json).unwrap_or_default();
            let _ = proxy.send_event(UserEvent::GoogleAccount(profile.clone(), parse_google_accounts(&text)));
        });
        if read.is_err() {
            self.account_probe = None;
        }
    }

    fn set_google_account(&mut self, profile: Option<String>, account: Option<GoogleAccount>) {
        self.account_probe = None;
        // Ignore a result for a persona that is no longer active.
        if profile == self.prefs.active_persona().profile_name() {
            self.prefs.google_hint = account.as_ref().map(|a| (a.name.clone(), a.email.clone()));
            self.google = account;
            self.sync_all_newtabs();
            self.sync_ui();
        }
        if std::mem::take(&mut self.probe_again) {
            self.schedule_account_probe(Duration::from_millis(500));
        }
    }

    /// Reopens every tab in the active persona's profile, keeping their order, pages and the
    /// active tab, the way switching profile in other browsers opens that profile's window.
    fn reopen_tabs_in_profile(&mut self) {
        if self.devtools.open {
            self.toggle_devtools();
        }
        let urls: Vec<Option<String>> =
            self.tabs.iter().map(|t| if t.is_newtab || t.url.is_empty() { None } else { Some(t.url.clone()) }).collect();
        let active = self.active;
        let old = std::mem::take(&mut self.tabs);
        self.spare = None;
        for url in urls {
            match self.build_tab(url.as_deref()) {
                Ok(tab) => self.tabs.push(tab),
                Err(_) => continue,
            }
        }
        if self.tabs.is_empty()
            && let Ok(tab) = self.build_tab(None)
        {
            self.tabs.push(tab);
        }
        self.active = active.min(self.tabs.len().saturating_sub(1));
        // Show the new active tab before the old ones go, so the window never flashes bare.
        self.show_active();
        for t in &old {
            let _ = self.thumbs_tx.send(thumbs::Request::Forget(t.id));
        }
        drop(old);
        let _ = self.proxy.send_event(UserEvent::Prewarm);
    }

    fn set_persona(&mut self, id: &str) {
        let old_profile = self.prefs.active_persona().profile_name();
        if !self.prefs.set_active_persona(id) {
            return;
        }
        let persona = self.prefs.active_persona().clone();
        let color = persona.parsed_color();
        let letter = persona.badge_letter();

        self.toolbar.set_persona_theme(self.dark, color, &letter);
        self.toolbar.set_background(self.prefs.bg_strip());
        self.popups.set_persona_theme(self.dark, color);

        let bg = self.newtab_color();
        let brand_key = if color == PersonaColor::Slate { "uzaki" } else { "cederic" };
        let js = format!(
            "window.__cedericBrand = {:?}; window.__cedericBrandName = {:?}; window.__cedericBrandColor = {:?}; window.__cedericSetBrand && window.__cedericSetBrand({:?});",
            brand_key, persona.name, color.hex_code(), brand_key
        );
        for tab in &mut self.tabs {
            if tab.newtab_bg {
                let _ = tab.view.set_background_color(bg);
            }
            if host_of(&tab.url) == Some(REAL_SEARCH_HOST) {
                tab.title = tab
                    .title
                    .replace("Brave", &persona.name)
                    .replace("Cederic", &persona.name)
                    .replace("Uzaki", &persona.name);
            }
            let _ = tab.view.evaluate_script(&js);
        }
        if let Some(spare) = &self.spare {
            if spare.newtab_bg {
                let _ = spare.view.set_background_color(bg);
            }
            let _ = spare.view.evaluate_script(&js);
        }
        if self.prefs.active_persona().profile_name() != old_profile {
            self.google = None;
            self.reopen_tabs_in_profile();
            self.schedule_account_probe(Duration::from_millis(800));
        }
        self.sync_all_newtabs();
        self.sync_ui();
        self.redraw();
    }

    fn set_brand(&mut self, brand: Brand) {
        let target_id = match brand {
            Brand::Cederic => "cederic",
            Brand::Uzaki => "uzaki",
        };
        if self.prefs.personas.iter().any(|p| p.id == target_id) {
            self.set_persona(target_id);
        } else {
            let color = PersonaColor::from(brand);
            let active_id = self.prefs.active_id().to_string();
            let cur_name = self.prefs.active_name().to_string();
            self.prefs.update_persona(&active_id, &cur_name, color.name(), None);
            self.set_persona(&active_id);
        }
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
            items.push(MenuItem::new("Copy image address", MenuAction::Copy(r.image.clone())));
            items.push(MenuItem::new("Set image as background", MenuAction::SetBackgroundUrl(r.image)));
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
        let fs_label = if self.fullscreen { "Exit Full Screen" } else { "Full Screen" };
        items.push(MenuItem::new(fs_label, MenuAction::ToggleFullscreen).hint("F11"));

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
            MenuAction::ClearSiteData => self.handle_chrome(ChromeCmd::ClearSiteData),
            MenuAction::RestartToUpdate => {
                // The exe at the path we started from is now the new version.
                if let Some(exe) = &self.exe_path
                    && std::process::Command::new(exe).spawn().is_ok()
                {
                    self.quit = true;
                }
            }
            MenuAction::OpenTab(url) => {
                let _ = self.open_tab(Some(url), None, true);
            }
            MenuAction::ToggleBookmark => self.toggle_bookmark(),
            MenuAction::DeleteBookmark(url) => {
                if self.prefs.remove_bookmark(&url) {
                    self.sync_all_newtabs();
                    self.sync_ui();
                }
            }
            MenuAction::SetBrand(brand) => self.set_brand(brand),
            MenuAction::SelectPersona(id) => self.set_persona(&id),
            MenuAction::OpenNewPersona => self.open_persona_editor(None),
            MenuAction::OpenManagePersonas => {
                let id = self.prefs.active_id().to_string();
                self.open_persona_editor(Some(&id));
            }
            MenuAction::ChooseBackground => self.choose_background(),
            MenuAction::SetBackgroundUrl(url) => {
                self.apply_background(Some(url.clone()), None);
                if let Ok(u) = serde_json::to_string(&url) {
                    let js = format!("document.dispatchEvent(new CustomEvent('cederic:act', {{ detail: {{ cmd: 'apply_bg', url: {u} }} }}))");
                    self.with_active(|v| v.evaluate_script(&js));
                }
            }
            MenuAction::ClearBackground => self.apply_background(None, None),
            MenuAction::ToggleFullscreen => self.handle_chrome(ChromeCmd::ToggleFullscreen),
            MenuAction::OpenSettings => self.open_settings(),
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

    // ---------- Fullscreen, Picture-in-Picture & DevTools ----------

    fn toggle_fullscreen(&mut self) {
        let next = !self.fullscreen;
        self.set_fullscreen(next, false);
    }

    fn set_fullscreen(&mut self, on: bool, is_html: bool) {
        if self.fullscreen == on {
            return;
        }
        self.fullscreen = on;
        self.html_fullscreen = on && is_html;
        if on {
            self.window.set_fullscreen(Some(Fullscreen::Borderless(None)));
        } else {
            self.window.set_fullscreen(None);
            if !is_html {
                let js = "if (document.fullscreenElement || document.webkitFullscreenElement) { (document.exitFullscreen || document.webkitExitFullscreen).call(document).catch(() => {}); }";
                self.with_active(|v| v.evaluate_script(js));
            }
        }
        self.close_menu();
        self.close_suggest();
        self.hide_preview();
        self.layout();
        self.sync_ui();
    }

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
        let js = format!("document.dispatchEvent(new CustomEvent('cederic:act', {{ detail: {{ cmd: 'pip', preferCtx: {prefer_ctx} }} }}))");
        let _ = self.tabs[i].view.evaluate_script(&js);
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
                    .with_user_agent(USER_AGENT)
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
            ChromeCmd::Tip { text, x, y } => self.show_tip(text, x, y),
            ChromeCmd::TipEnd => self.hide_tip(),
            ChromeCmd::Dismiss => {
                if self.fullscreen {
                    self.set_fullscreen(false, false);
                }
                self.close_menu();
            }
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
            // Only the search history: cookies stay, so saved Google (and other) sign-ins survive.
            ChromeCmd::ClearHistory => {
                self.history.clear();
                self.sync_ui();
            }
            ChromeCmd::ClearSiteData => {
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
            ChromeCmd::ToggleFullscreen => self.toggle_fullscreen(),
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
            Pick::Modal(index) => {
                let p = &self.prefs.permissions;
                let (mut cam, mut mic, mut notif, mut geo) = (p.camera, p.microphone, p.notifications, p.geolocation);
                match index {
                    0 => cam = !cam,
                    1 => mic = !mic,
                    2 => notif = !notif,
                    3 => geo = !geo,
                    _ => {
                        self.hide_modal();
                        self.refocus_page();
                        return;
                    }
                }
                self.set_permissions(cam, notif, mic, geo);
                self.show_settings();
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
            Shortcut::Fullscreen => self.handle_chrome(ChromeCmd::ToggleFullscreen),
            Shortcut::Escape => {
                if self.fullscreen {
                    self.set_fullscreen(false, false);
                }
                self.close_menu();
                self.toolbar.blur_address();
                self.window.request_redraw();
            }
            Shortcut::Dismiss => {
                // A click or focus in the page: close menus and stop editing the address bar.
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
        if let NewTabCmd::SetBackground { data, strip } = cmd {
            let d = (!data.trim().is_empty()).then_some(data);
            let s = (!strip.trim().is_empty()).then_some(strip);
            self.apply_background(d, s.as_deref());
            return;
        }
        if let Some(spare) = &self.spare
            && spare.id == id
        {
            if matches!(cmd, NewTabCmd::Ready) {
                Self::push_newtab_state(&spare.view, &self.prefs, &self.history);
            }
            return;
        }
        let Some(i) = self.index_of(id) else { return };
        if !self.tabs[i].is_newtab {
            return;
        }
        match cmd {
            NewTabCmd::Ready => {
                Self::push_newtab_state(&self.tabs[i].view, &self.prefs, &self.history);
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
            NewTabCmd::SelectPersona { id } => self.set_persona(&id),
            NewTabCmd::CreatePersona { name, color, pfp } => {
                if let Ok(nid) = self.prefs.create_persona(&name, &color, pfp) {
                    self.set_persona(&nid);
                }
            }
            NewTabCmd::UpdatePersona { id, name, color, pfp } => {
                if self.prefs.update_persona(&id, &name, &color, Some(pfp)) {
                    if id == self.prefs.active_persona_id {
                        self.set_persona(&id);
                    } else {
                        self.sync_all_newtabs();
                        self.sync_ui();
                    }
                }
            }
            NewTabCmd::DeletePersona { id } => {
                if self.prefs.delete_persona(&id).is_ok() {
                    let active_id = self.prefs.active_id().to_string();
                    self.set_persona(&active_id);
                }
            }
            NewTabCmd::SetPermissions { camera, notifications, microphone, geolocation } => {
                self.set_permissions(camera, notifications, microphone, geolocation);
            }
            NewTabCmd::SetBrand { brand } => self.set_brand(brand),
            NewTabCmd::SetBackground { .. } => {}
            NewTabCmd::OpenSettings => self.show_settings(),
            NewTabCmd::LinkGoogle => {
                let _ = self.open_tab(Some(GOOGLE_SIGN_IN_URL.into()), None, true);
            }
            NewTabCmd::CheckGoogle => self.schedule_account_probe(Duration::from_millis(100)),
            NewTabCmd::FinishSetup => {
                // Setup is mandatory: it only finishes once a Google account is linked.
                if self.google.is_none() {
                    return;
                }
                self.prefs.finish_setup();
                self.sync_all_newtabs();
            }
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
                    // A dialog is open: swap in the fresh snapshot behind it.
                    if i == self.active && self.popups.is_open(PopupKind::Scrim) {
                        self.show_scrim(false);
                    }
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
            UserEvent::FullscreenState(id, on) => {
                if self.active_tab().map(|t| t.id) == Some(id) {
                    self.set_fullscreen(on, true);
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
                    let brand_name = self.prefs.active_name();
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
            UserEvent::UpdateReady(version) => {
                self.update_ready = Some(version);
                self.sync_ui();
            }
            UserEvent::ProbeAccount => {
                self.probe_scheduled = false;
                self.probe_google_account();
            }
            UserEvent::AccountProbeLoaded => self.account_probe_loaded(),
            UserEvent::GoogleAccount(profile, account) => self.set_google_account(profile, account),
            UserEvent::Load(id, ev, url) => {
                // Signing in or out of Google finishes on a Google page. Wait until the sign-in and
                // challenge steps (passwords, passkeys, 2-step) are over before checking.
                if matches!(ev, PageLoadEvent::Finished)
                    && host_of(&url).is_some_and(|h| h == "google.com" || h.ends_with(".google.com"))
                    && !["/signin", "/challenge", "/v3/", "/ServiceLogin", "/InteractiveLogin"].iter().any(|p| url.contains(p))
                {
                    self.schedule_account_probe(Duration::from_millis(1500));
                }
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
                    if !tab.url.contains("youtube.com") && !tab.url.contains("youtu.be") {
                        let _ = self.thumbs_tx.send(thumbs::Request::Capture(id));
                    }
                    let proxy = self.proxy.clone();
                    let _ = tab.view.evaluate_script_with_callback(FAVICON_SCRIPT, move |res| {
                        if let Some(icon) = parse_favicon(&res) {
                            let _ = proxy.send_event(UserEvent::Favicon(id, icon));
                        }
                    });
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
                let color = self.prefs.active_color();
                let letter = self.prefs.active_badge_letter();
                self.toolbar.set_persona_theme(self.dark, color, &letter);
                self.popups.set_persona_theme(self.dark, color);
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
                if !self.fullscreen && (p.y < CHROME_HEIGHT || self.toolbar.captures_mouse()) {
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
                if !self.fullscreen {
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
        if self.fullscreen {
            return;
        }
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
        let (w_u32, h_u32) = (w.get(), h.get());
        if self.surface_size != (w_u32, h_u32) {
            if self.surface.resize(w, h).is_err() {
                return;
            }
            self.surface_size = (w_u32, h_u32);
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
            let (strip, _, _) = self.prefs.active_color().surface_tints(self.dark);
            let color = ((strip.0 as u32) << 16) | ((strip.1 as u32) << 8) | (strip.2 as u32);
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

fn search_host_for_persona(name: &str) -> String {
    let slug: String = name
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    if slug.is_empty() {
        "search.persona.com".into()
    } else {
        format!("search.{slug}.com")
    }
}

/// The address-bar text for `url`; Brave Search is shown as Persona's own search host.
fn display_url(url: &str, brand: impl AsRef<str>) -> String {
    if url.starts_with("about:") || url.starts_with("data:") {
        return String::new();
    }
    match url.split_once("://") {
        Some((scheme, rest)) if host_of(url) == Some(REAL_SEARCH_HOST) => {
            let shown_host = search_host_for_persona(brand.as_ref());
            format!("{scheme}://{shown_host}{}", &rest[REAL_SEARCH_HOST.len()..])
        }
        _ => url.to_owned(),
    }
}

/// Maps the displayed search host back to the real one when the user edits and submits it.
fn unmask(input: &str) -> String {
    let mut res = input
        .replace(Brand::Cederic.search_host(), REAL_SEARCH_HOST)
        .replace(Brand::Uzaki.search_host(), REAL_SEARCH_HOST)
        .replace("search.persona.com", REAL_SEARCH_HOST);
    if let Some((scheme, rest)) = res.split_once("://")
        && let Some((host, path_and_query)) = rest.split_once('/')
        && host.starts_with("search.")
        && host.ends_with(".com")
    {
        res = format!("{scheme}://{REAL_SEARCH_HOST}/{path_and_query}");
    }
    res
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then_some(host)
}

fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Cederic")
}

fn base64_encode(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[((n >> 18) & 0x3F) as usize] as char);
        out.push(T[((n >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 { T[((n >> 6) & 0x3F) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 0x3F) as usize] as char } else { '=' });
    }
    out
}

fn load_app_icon() -> Option<WinIcon> {
    let pm = Pixmap::decode_png(include_bytes!("ui/logo_64.png")).ok()?;
    let (w, h) = (pm.width(), pm.height());
    let mut rgba = pm.data().to_vec();
    for p in rgba.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a > 0 && a < 255 {
            p[0] = ((p[0] as u32 * 255 + a / 2) / a).min(255) as u8;
            p[1] = ((p[1] as u32 * 255 + a / 2) / a).min(255) as u8;
            p[2] = ((p[2] as u32 * 255 + a / 2) / a).min(255) as u8;
        }
    }
    WinIcon::from_rgba(rgba, w, h).ok()
}

fn main() -> wry::Result<()> {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    update::cleanup();
    {
        let proxy = proxy.clone();
        update::spawn(move |v| {
            let _ = proxy.send_event(UserEvent::UpdateReady(v.to_string()));
        });
    }

    let dir = data_dir();
    // Release builds abort on panic with no message; keep the reason and location on disk so a
    // crash can be traced. Appends to %LOCALAPPDATA%\Persona\crash.log (no page data in it).
    let crash_log = dir.join("crash.log");
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&crash_log) {
            let _ = writeln!(f, "[{secs}] {} {info}", env!("CARGO_PKG_VERSION"));
        }
    }));
    let prefs = Prefs::open(&dir);
    let brand = prefs.brand();
    let color = prefs.active_color();
    let letter = prefs.active_badge_letter();

    let window = Rc::new(
        WindowBuilder::new()
        .with_title("Persona")
        .with_window_icon(load_app_icon())
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
    let mut toolbar = Toolbar::new_with_brand(dark, brand, scale as f32, size.width as f32);
    toolbar.set_persona_theme(dark, color, &letter);
    toolbar.set_background(prefs.bg_strip());
    let softbuffer_ctx = softbuffer::Context::new(window.clone()).expect("failed to create drawing context");
    let surface = softbuffer::Surface::new(&softbuffer_ctx, window.clone()).expect("failed to create drawing surface");

    let mut popups = Popups::new(&event_loop, &window, dark, brand);
    popups.set_persona_theme(dark, color);

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

    let allow_camera = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(prefs.permissions.camera));
    let allow_notifications = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(prefs.permissions.notifications));
    let allow_microphone = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(prefs.permissions.microphone));
    let allow_geolocation = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(prefs.permissions.geolocation));

    let mut browser = Browser {
        window,
        toolbar,
        surface,
        surface_size: (0, 0),
        frame: Pixmap::new(1, 1).expect("1x1 pixmap"),
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
        fullscreen: false,
        html_fullscreen: false,
        last_title: String::new(),
        google: None,
        account_probe: None,
        probe_again: false,
        probe_scheduled: false,
        update_ready: None,
        exe_path: std::env::current_exe().ok(),
        allow_camera,
        allow_notifications,
        allow_microphone,
        allow_geolocation,
    };
    // Until setup is finished the browser only opens the New Tab page, where setup runs.
    let start = std::env::args().nth(1).filter(|_| browser.prefs.setup_done).and_then(|a| {
        let resolved = input::resolve_with_info(&unmask(&a))?;
        browser.history.record(&resolved.query, &resolved.url, resolved.engine, resolved.is_search);
        Some(resolved.url)
    });
    browser.open_tab(start, None, true)?;
    browser.layout();
    browser.schedule_account_probe(Duration::from_millis(1500));

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
            // Search history is per session; cookies are kept so accounts stay signed in.
            browser.history.clear();
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
    fn setup_screen_state_persists_and_skips_existing_users() {
        let tmp = |tag: &str| {
            std::env::temp_dir().join(format!(
                "persona_setup_test_{tag}_{}",
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ))
        };
        // A fresh profile shows the setup screen until it is finished, and remembers that.
        let dir = tmp("new");
        let mut prefs = Prefs::open(&dir);
        assert!(!prefs.setup_done);
        assert!(!Prefs::open(&dir).setup_done);
        prefs.finish_setup();
        assert!(Prefs::open(&dir).setup_done);
        let _ = std::fs::remove_dir_all(&dir);

        // Prefs written before the setup screen existed already have personas: not shown again.
        let dir = tmp("old");
        std::fs::create_dir_all(&dir).unwrap();
        let p = Prefs::open(&dir);
        p.save();
        let json = std::fs::read_to_string(dir.join("prefs.json")).unwrap();
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v.as_object_mut().unwrap().remove("setup_done");
        std::fs::write(dir.join("prefs.json"), v.to_string()).unwrap();
        assert!(Prefs::open(&dir).setup_done);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_personas_are_removed_and_fresh_profiles_start_with_one() {
        // Untouched Uzaki goes; an untouched lone Cederic means setup still has to run.
        let mut p = vec![PersonaProfile::new_cederic(), PersonaProfile::new_uzaki()];
        assert!(migrate_test_personas(&mut p));
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].id, "cederic");
        // A renamed persona is the user's: nothing is removed and setup is not forced.
        let mut mine = PersonaProfile::new_cederic();
        mine.name = "Work".into();
        let mut p = vec![mine, PersonaProfile::new_uzaki()];
        assert!(!migrate_test_personas(&mut p));
        assert_eq!(p.len(), 1, "an untouched Uzaki still goes");
        // A used Uzaki stays.
        let mut used = PersonaProfile::new_uzaki();
        used.bookmarks.push(Bookmark { title: "x".into(), url: "https://example.com/".into() });
        let mut p = vec![PersonaProfile::new_cederic(), used];
        migrate_test_personas(&mut p);
        assert_eq!(p.len(), 2);
        // A fresh profile gets exactly one persona, on its own WebView2 profile.
        let fresh = PersonaProfile::new_main();
        assert_eq!(fresh.profile_name().as_deref(), Some("persona_main"));
    }

    #[test]
    fn other_personas_backgrounds_load_when_selected() {
        let dir = std::env::temp_dir().join(format!(
            "persona_bg_test_{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("background_uzaki.txt"), "data:image/png;base64,AAAA").unwrap();
        let mut prefs = Prefs::open(&dir);
        // Only the active persona's background is read at startup.
        assert!(prefs.background_for("uzaki").is_none());
        assert!(prefs.set_active_persona("uzaki"));
        assert_eq!(prefs.background(), Some("data:image/png;base64,AAAA"));
        let _ = std::fs::remove_dir_all(&dir);
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
        assert_eq!(prefs.brand(), Brand::Cederic);
        assert!(prefs.bookmarks().is_empty());

        assert!(prefs.toggle_bookmark("Rust Lang", "https://www.rust-lang.org/"));
        assert!(prefs.is_bookmarked("https://www.rust-lang.org/"));

        // Switch to Uzaki: bookmarks are isolated per persona!
        prefs.set_brand(Brand::Uzaki);
        assert_eq!(prefs.brand(), Brand::Uzaki);
        assert!(prefs.bookmarks().is_empty());
        assert!(!prefs.is_bookmarked("https://www.rust-lang.org/"));

        // Bookmark in Uzaki
        assert!(prefs.toggle_bookmark("Uzaki Site", "https://example.com/uzaki"));
        assert!(prefs.is_bookmarked("https://example.com/uzaki"));

        // Switch back to Cederic: Cederic still has its bookmark, not Uzaki's
        prefs.set_brand(Brand::Cederic);
        assert_eq!(prefs.bookmarks().len(), 1);
        assert_eq!(prefs.bookmarks()[0].title, "Rust Lang");
        assert!(prefs.is_bookmarked("https://www.rust-lang.org/"));
        assert!(!prefs.is_bookmarked("https://example.com/uzaki"));

        let reloaded = Prefs::open(&dir);
        assert_eq!(reloaded.brand(), Brand::Cederic);
        assert_eq!(reloaded.bookmarks().len(), 1);
        assert_eq!(reloaded.bookmarks()[0].title, "Rust Lang");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shortcut_parsing_includes_pip_and_bookmarks() {
        assert!(matches!(Shortcut::parse("pip"), Some(Shortcut::Pip)));
        assert!(matches!(Shortcut::parse("bookmark"), Some(Shortcut::Bookmark)));
        assert!(matches!(Shortcut::parse("bookmarks"), Some(Shortcut::Bookmarks)));
        assert!(matches!(Shortcut::parse("fullscreen"), Some(Shortcut::Fullscreen)));
        assert!(matches!(Shortcut::parse("escape"), Some(Shortcut::Escape)));
    }

    #[test]
    fn prefs_per_persona_custom_background() {
        let dir = std::env::temp_dir().join(format!(
            "cederic_bg_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut prefs = Prefs::open(&dir);
        assert_eq!(prefs.brand(), Brand::Cederic);
        assert_eq!(prefs.background(), None);

        // Set Cederic background
        prefs.set_background(Brand::Cederic, Some("https://example.com/cederic.jpg".into()), None);
        assert_eq!(prefs.background(), Some("https://example.com/cederic.jpg"));

        // Switch to Uzaki: should not have a background yet
        prefs.set_brand(Brand::Uzaki);
        assert_eq!(prefs.background(), None);

        // Set Uzaki background
        prefs.set_background(Brand::Uzaki, Some("https://example.com/uzaki.png".into()), None);
        assert_eq!(prefs.background(), Some("https://example.com/uzaki.png"));

        // Switch back to Cederic: should still have Cederic background
        prefs.set_brand(Brand::Cederic);
        assert_eq!(prefs.background(), Some("https://example.com/cederic.jpg"));

        // Reload prefs from disk: both backgrounds must be preserved
        let mut reloaded = Prefs::open(&dir);
        // Only the active persona's background is read at startup; the other loads on demand.
        reloaded.ensure_background("uzaki");
        assert_eq!(reloaded.background_for(Brand::Cederic), Some("https://example.com/cederic.jpg"));
        assert_eq!(reloaded.background_for(Brand::Uzaki), Some("https://example.com/uzaki.png"));

        // Clear Uzaki background: Cederic must remain untouched
        reloaded.set_background(Brand::Uzaki, None, None);
        assert_eq!(reloaded.background_for(Brand::Uzaki), None);
        assert_eq!(reloaded.background_for(Brand::Cederic), Some("https://example.com/cederic.jpg"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prefs_persona_creation_limits_and_deletion() {
        let dir = std::env::temp_dir().join(format!(
            "persona_mgmt_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut prefs = Prefs::open(&dir);
        // Initially 2 base personas: Cederic and Uzaki
        assert_eq!(prefs.personas.len(), 2);
        assert_eq!(prefs.personas[0].name, "Cederic");
        assert_eq!(prefs.personas[1].name, "Uzaki");

        // Create 3rd, 4th, 5th persona
        let p3 = prefs.create_persona("Work", "emerald", Some("data:image/png;base64,abc".into())).unwrap();
        let p4 = prefs.create_persona("Gaming", "violet", None).unwrap();
        let p5 = prefs.create_persona("Secret", "ruby", None).unwrap();
        assert_eq!(prefs.personas.len(), 5);

        // Max 5 personas enforced: creating a 6th must fail
        let p6_err = prefs.create_persona("Overflow", "cyan", None);
        assert!(p6_err.is_err());
        assert_eq!(prefs.personas.len(), 5);

        // Active persona should be p5
        assert_eq!(prefs.active_id(), &p5);
        assert_eq!(prefs.active_name(), "Secret");
        assert_eq!(prefs.active_color(), PersonaColor::Ruby);

        // Edit p3
        assert!(prefs.update_persona(&p3, "Work & Code", "cyan", Some(None)));
        let p3_prof = prefs.personas.iter().find(|p| p.id == p3).unwrap();
        assert_eq!(p3_prof.name, "Work & Code");
        assert_eq!(p3_prof.color, "cyan");
        assert_eq!(p3_prof.pfp, None);

        // User can delete base personas if they want!
        assert!(prefs.delete_persona("cederic").is_ok());
        assert_eq!(prefs.personas.len(), 4);
        assert!(!prefs.personas.iter().any(|p| p.id == "cederic"));

        assert!(prefs.delete_persona("uzaki").is_ok());
        assert_eq!(prefs.personas.len(), 3);
        assert!(!prefs.personas.iter().any(|p| p.id == "uzaki"));

        // Delete down to 1 persona
        assert!(prefs.delete_persona(&p4).is_ok());
        assert!(prefs.delete_persona(&p5).is_ok());
        assert_eq!(prefs.personas.len(), 1);

        // Cannot delete the only remaining persona!
        let last_id = prefs.personas[0].id.clone();
        let del_err = prefs.delete_persona(&last_id);
        assert!(del_err.is_err());
        assert_eq!(prefs.personas.len(), 1);

        // Persistence check
        let reloaded = Prefs::open(&dir);
        assert_eq!(reloaded.personas.len(), 1);
        assert_eq!(reloaded.personas[0].name, "Work & Code");
        assert_eq!(reloaded.personas[0].color, "cyan");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_google_account_list() {
        let text = r#"["gaia.l.a.r",[["gaia.l.a",1,"Merik Venne","merik@example.com","https://lh3.googleusercontent.com/a/ACg8ocK=s48-c",1,1,0,null,1,"1234",null,null,null,0,1]]]"#;
        let a = parse_google_accounts(text).expect("one account");
        assert_eq!(a.name, "Merik Venne");
        assert_eq!(a.email, "merik@example.com");
        assert_eq!(a.photo, "https://lh3.googleusercontent.com/a/ACg8ocK=s96-c");
        // Signed out ([14] = 1) or invalid ([9] = 0) sessions do not count.
        let out = r#"["gaia.l.a.r",[["gaia.l.a",1,"X","x@example.com","",1,1,0,null,1,"1",null,null,null,1,1]]]"#;
        assert_eq!(parse_google_accounts(out), None);
        let invalid = r#"["gaia.l.a.r",[["gaia.l.a",1,"X","x@example.com","",1,1,0,null,0,"1"]]]"#;
        assert_eq!(parse_google_accounts(invalid), None);
        // No accounts, junk, and photos from other hosts.
        assert_eq!(parse_google_accounts(r#"["gaia.l.a.r",[]]"#), None);
        assert_eq!(parse_google_accounts("<html>"), None);
        let evil = r#"["gaia.l.a.r",[["gaia.l.a",1,"E","e@example.com","https://evil.example/p.png",1,1,0,null,1,"1"]]]"#;
        assert_eq!(parse_google_accounts(evil).unwrap().photo, "");
        let photo_plain = sized_google_photo("https://lh3.googleusercontent.com/a/abc");
        assert_eq!(photo_plain, "https://lh3.googleusercontent.com/a/abc=s96-c");
    }

    #[test]
    fn persona_profiles_are_separate() {
        let base = PersonaProfile::new_cederic();
        assert_eq!(base.profile_name(), None);
        let mut other = PersonaProfile::new_uzaki();
        assert_eq!(other.profile_name().as_deref(), Some("persona_uzaki"));
        other.id = "p_17/../x y".into();
        assert_eq!(other.profile_name().as_deref(), Some("persona_p_17xy"));
    }

    #[test]
    fn prefs_permissions_persistence() {
        let dir = std::env::temp_dir().join(format!(
            "persona_perms_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut prefs = Prefs::open(&dir);
        // Defaults: camera=true, notifications=true, microphone=true, geolocation=false
        assert!(prefs.permissions.camera);
        assert!(prefs.permissions.notifications);
        assert!(prefs.permissions.microphone);
        assert!(!prefs.permissions.geolocation);

        // Update permissions
        prefs.set_permissions(false, false, false, true);
        assert!(!prefs.permissions.camera);
        assert!(!prefs.permissions.notifications);
        assert!(!prefs.permissions.microphone);
        assert!(prefs.permissions.geolocation);

        // Verify disk persistence
        let reloaded = Prefs::open(&dir);
        assert!(!reloaded.permissions.camera);
        assert!(!reloaded.permissions.notifications);
        assert!(!reloaded.permissions.microphone);
        assert!(reloaded.permissions.geolocation);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
