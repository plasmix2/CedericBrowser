# Persona

A lean Windows browser written in Rust. The browser itself forbids `unsafe` code and ships as a single ~4 MB executable (`Persona.exe`); web pages are rendered by the system's WebView2 engine.

> **AI agents:** read [AGENTS.md](AGENTS.md) before changing anything.

## Build & run

Requires Windows 10/11 (WebView2 is built in) and a Rust toolchain.

```
cargo run --release              # opens a new tab
cargo run --release -- example.com
cargo test
```

## Releases & updates

Pushing a version tag builds Persona on GitHub Actions and publishes it as a release
(`.github/workflows/release.yml`). The tag must match `version` in `Cargo.toml`:

```
git tag v0.2.0
git push origin v0.2.0
```

Installed copies check this repository's releases about 15 seconds after starting. If there is a
newer version, they download it, check its size and SHA-256 digest, and put it in place of the
running exe. A green dot on the persona button means an update is ready, and the persona menu
offers **Restart to update**. Builds run from a cargo `target` folder never update themselves.
Set `PERSONA_NO_UPDATE=1` to turn updates off.

## Features

- **Tabs in the title bar**: frameless window with custom window controls, animated tab open/close, drag to reorder, and hover cards showing a snapshot of the page.
- **Fast tabs**: a hidden new tab is always pre-loaded so Ctrl+T is instant. Background tabs are suspended and set to WebView2's low-memory level, and closing a tab frees its memory.
- **Address bar**: URLs, `localhost:3000`, IPs, or a search, with live suggestions. `javascript:` and unknown schemes are refused, and non-local `http://` is upgraded to `https://`.
- **Cederic Search**: searches go to Brave Search, shown in the browser as `search.cederic.com` with Cederic's name and logo.
- **Right-click menus** on tabs and pages: open link in new tab, copy link or selection, search for selection, Picture-in-Picture, view source, inspect.
- **Picture-in-Picture** (toolbar button, right-click menu, or Alt+P): pops out the active video on a page into a floating always-on-top player and keeps playback running when switching tabs.
- **Bookmarks** (star button or Ctrl+D to bookmark/unbookmark, bookmarks button or Ctrl+B for saved sites menu, and bookmark pills on the New Tab page), persisted in `%LOCALAPPDATA%\Cederic\prefs.json`.
- **Theme & Persona Switcher** (palette button in toolbar or New Tab switcher): Cederic (Orange) by default, with one-click switching to **Uzaki** (Slate Blue theme and Uzaki branding across the New Tab page, window title, tab badge, and search wordmark/host).
- **DevTools docked** on the right (F12), with a draggable splitter.
- **Search history** (clock button or Ctrl+H), encrypted on disk in `%LOCALAPPDATA%\Cederic\history.enc`.
- **Privacy defaults**: autofill off, site permission requests (location, camera, …) denied, pop-ups open as tabs, and pages can only send a fixed set of commands to the browser. Browser flags request DNS-over-HTTPS (Cloudflare), Encrypted Client Hello and WebRTC IP-leak protection; whether WebView2 honours each flag has not been verified.

| Shortcut | Action |
|---|---|
| Ctrl+T / Ctrl+W | New / close tab |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
| Ctrl+L | Address bar |
| Ctrl+D | Bookmark / unbookmark current page |
| Ctrl+B | Saved bookmarks menu |
| Ctrl+H | Search history |
| Ctrl+R, F5 | Reload |
| Alt+Left / Alt+Right | Back / forward |
| Alt+P | Picture-in-Picture |
| F12 | DevTools |

## Layout

See the file table in [AGENTS.md](AGENTS.md#project-overview).
