# Cederic

A lean browser shell written in Rust. It forbids `unsafe` code, ships as a single ~1 MB executable, and uses the system's WebView2 engine to render pages.

## Build & run

```
cargo run --release              # opens a new tab
cargo run --release -- example.com
```

## Features

- Tabs: middle-click to close, double-click the strip to open one. Each tab is its own WebView, and its memory is freed when you close it.
- Background tabs are suspended (`BackgroundThrottlingPolicy::Suspend`).
- Brave-style frameless title bar: tabs sit in the title bar next to custom window controls, with animated tab open/close, a page-load progress bar and favicons.
- A pre-loaded hidden new tab, so Ctrl+T opens instantly. Hidden tabs drop to WebView2's low-memory level.
- Smart address bar: URLs, `localhost:3000`, IPs, or an encrypted Brave Search query. `javascript:` and other unknown schemes are refused, and non-local `http://` navigations are automatically upgraded to `https://`. To switch to Google, change `SEARCH` in `src/input.rs`.
- Wi-Fi eavesdrop protection: enforces HTTPS-Only upgrades, built-in DNS-over-HTTPS (DoH via Cloudflare so local Wi-Fi routers cannot see DNS lookups), Encrypted Client Hello (ECH), Post-Quantum Kyber key exchange, WebRTC local-IP leak prevention, and blocks geolocation/sensor permissions.
- Encrypted search history & moments: automatically records searches and the exact moment they occurred (both UTC civil timestamps and local relative/exact timestamps), encrypted at rest in `%LOCALAPPDATA%\Cederic\history.enc` using pure safe-Rust ChaCha20-Poly1305 (RFC 8439) and SHA-256 key derivation.
- Pop-ups and `target=_blank` links open as tabs, not new windows.
- Web content can only send a fixed whitelist of shortcut commands to the browser.
- Autofill is off. The profile is stored in `%LOCALAPPDATA%\Cederic`.

| Shortcut | Action |
|---|---|
| Ctrl+T / Ctrl+W | New / close tab |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
| Ctrl+L | Address bar |
| Ctrl+H | Encrypted search history drawer |
| Ctrl+R, F5 | Reload |
| Alt+Left / Alt+Right | Back / forward |
| F12 | DevTools |

## Layout

- `src/main.rs`: window, tab management, navigation/permission guards, event loop
- `src/input.rs`: turns address-bar input into an encrypted URL, upgrades `http://` to `https://`, and extracts search queries (with tests)
- `src/crypto.rs`: pure safe-Rust ChaCha20-Poly1305 AEAD and SHA-256 for encrypting history at rest (with RFC 8439 / NIST tests)
- `src/history.rs`: encrypted search history store with exact moment timestamps (with tests)
- `src/ui/chrome.html`: tab strip, toolbar, Wi-Fi encryption shield, and expandable search history drawer
- `src/ui/newtab.html`: start page with encrypted search bar and saved search history & moments vault
