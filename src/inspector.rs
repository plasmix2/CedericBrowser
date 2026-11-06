//! Persona's own DevTools, drawn natively by ANVL in the pane beside the page.
//!
//! A worker thread hooks onto the tab's page through the engine's DevTools protocol (the same
//! loopback debugging port the tab snapshots use) and streams what happens there: console
//! messages and errors, network requests, and the DOM tree. The `Panel` shows it in three tabs
//! (Elements, Console, Network) and sends commands back: evaluate a line of JavaScript, expand a
//! node, highlight an element on the page.

use std::{
    collections::HashMap,
    net::TcpStream,
    sync::mpsc,
    time::{Duration, Instant},
};

use anvil_ui::{Editor, InputKind};
use serde_json::{Value, json};
use tao::{
    event::ElementState,
    keyboard::{Key, ModifiersState},
    window::CursorIcon,
};
use tiny_skia::Pixmap;
use tungstenite::{Message, WebSocket};

use crate::{
    anvil::{self, Engine, Painter, Weight},
    gfx::{Icon, PersonaColor, Rgba, Theme},
};

// ---------- protocol worker ----------

/// What the panel asks the worker to do.
pub enum Cmd {
    /// Hook onto this page target on the debugging port.
    Attach { port: u16, target: String },
    Detach,
    Eval(String),
    /// Load the children of a DOM node.
    Expand(i64),
    /// Outline a DOM node on the page, or remove the outline.
    Highlight(Option<i64>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Log,
    Info,
    Warn,
    Error,
    /// A line typed at the console prompt.
    Input,
    /// What it evaluated to.
    Result,
}

#[derive(Clone, Debug, Default)]
pub struct Node {
    pub id: i64,
    /// `div`, `#text`, `<!DOCTYPE html>`...
    pub name: String,
    /// The interesting attributes, already formatted (` id="a" class="b"`), or a text node's text.
    pub detail: String,
    pub child_count: usize,
    /// Children the engine sent along (it sends a few levels at once).
    pub children: Option<Vec<Node>>,
}

/// What the worker reports.
pub enum Ev {
    Attached,
    Failed(String),
    /// The page navigated or reloaded: start the console, network list and DOM afresh.
    Navigated,
    Console(Level, String),
    Request { id: String, method: String, url: String, kind: String },
    Response { id: String, status: u16, kind: String },
    Done { id: String, bytes: f64, error: Option<String> },
    /// The document (parent `None`) or the children of a node.
    Nodes { parent: Option<i64>, nodes: Vec<Node> },
}

/// Starts the worker. `on_event` runs on the worker thread.
pub fn spawn(on_event: impl Fn(Ev) + Send + 'static) -> mpsc::Sender<Cmd> {
    let (tx, rx) = mpsc::channel::<Cmd>();
    std::thread::spawn(move || {
        let mut session: Option<Session> = None;
        loop {
            // Commands: wait for one while detached, otherwise take what is queued.
            let mut cmds = Vec::new();
            if session.is_none() {
                match rx.recv() {
                    Ok(c) => cmds.push(c),
                    Err(_) => return,
                }
            }
            loop {
                match rx.try_recv() {
                    Ok(c) => cmds.push(c),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                }
            }
            for cmd in cmds {
                match cmd {
                    Cmd::Attach { port, target } => {
                        session = None;
                        match Session::attach(port, &target) {
                            Some(s) => {
                                session = Some(s);
                                on_event(Ev::Attached);
                            }
                            None => on_event(Ev::Failed("Could not connect to this page.".into())),
                        }
                    }
                    Cmd::Detach => session = None,
                    Cmd::Eval(expr) => {
                        if let Some(s) = &mut session {
                            s.call(
                                "Runtime.evaluate",
                                json!({ "expression": expr, "includeCommandLineAPI": true, "replMode": true, "generatePreview": false, "userGesture": true }),
                                Pending::Eval,
                            );
                        }
                    }
                    Cmd::Expand(id) => {
                        if let Some(s) = &mut session {
                            s.call("DOM.requestChildNodes", json!({ "nodeId": id, "depth": 1 }), Pending::Ignore);
                        }
                    }
                    Cmd::Highlight(node) => {
                        if let Some(s) = &mut session {
                            match node {
                                Some(id) => s.call(
                                    "Overlay.highlightNode",
                                    json!({ "nodeId": id, "highlightConfig": {
                                        "contentColor": { "r": 255, "g": 122, "b": 61, "a": 0.28 },
                                        "paddingColor": { "r": 255, "g": 190, "b": 120, "a": 0.25 },
                                        "marginColor": { "r": 255, "g": 220, "b": 160, "a": 0.2 },
                                        "showInfo": true
                                    } }),
                                    Pending::Ignore,
                                ),
                                None => s.call("Overlay.hideHighlight", json!({}), Pending::Ignore),
                            }
                        }
                    }
                }
            }
            // Messages from the page.
            let Some(s) = &mut session else { continue };
            match s.ws.read() {
                Ok(Message::Text(text)) => {
                    if let Ok(v) = serde_json::from_str::<Value>(text.as_str()) {
                        s.handle(&v, &on_event);
                    }
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(_) => {
                    session = None;
                    on_event(Ev::Failed("The page went away.".into()));
                }
            }
        }
    });
    tx
}

enum Pending {
    Eval,
    Document,
    Ignore,
}

struct Session {
    ws: WebSocket<TcpStream>,
    next: u64,
    pending: HashMap<u64, Pending>,
}

impl Session {
    fn attach(port: u16, target: &str) -> Option<Self> {
        let stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
        stream.set_write_timeout(Some(Duration::from_millis(1500))).ok()?;
        let url = format!("ws://127.0.0.1:{port}/devtools/page/{target}");
        let (ws, _) = tungstenite::client(url.as_str(), stream).ok()?;
        // Short reads, so commands from the panel are picked up promptly.
        ws.get_ref().set_read_timeout(Some(Duration::from_millis(40))).ok()?;
        let mut s = Self { ws, next: 1, pending: HashMap::new() };
        for m in ["Runtime.enable", "Log.enable", "Network.enable", "DOM.enable", "Overlay.enable"] {
            s.call(m, json!({}), Pending::Ignore);
        }
        s.call("DOM.getDocument", json!({ "depth": 3 }), Pending::Document);
        Some(s)
    }

    fn call(&mut self, method: &str, params: Value, pending: Pending) {
        let id = self.next;
        self.next += 1;
        self.pending.insert(id, pending);
        let _ = self.ws.send(Message::text(json!({ "id": id, "method": method, "params": params }).to_string()));
    }

    fn handle(&mut self, v: &Value, on: &impl Fn(Ev)) {
        if let Some(id) = v["id"].as_u64() {
            match self.pending.remove(&id) {
                Some(Pending::Eval) => {
                    let r = &v["result"];
                    if let Some(ex) = r.get("exceptionDetails") {
                        let text = ex["exception"]["description"].as_str().or(ex["text"].as_str()).unwrap_or("Error").to_owned();
                        on(Ev::Console(Level::Error, text));
                    } else {
                        on(Ev::Console(Level::Result, describe(&r["result"])));
                    }
                }
                Some(Pending::Document) => {
                    if let Some(root) = v["result"]["root"].as_object() {
                        let root = parse_node(&Value::Object(root.clone()));
                        on(Ev::Nodes { parent: None, nodes: root.children.unwrap_or_default() });
                    }
                }
                _ => {}
            }
            return;
        }
        let p = &v["params"];
        let s = |k: &str| p[k].as_str().unwrap_or_default().to_owned();
        match v["method"].as_str().unwrap_or_default() {
            "Runtime.consoleAPICalled" => {
                let level = match p["type"].as_str().unwrap_or("log") {
                    "error" | "assert" => Level::Error,
                    "warning" => Level::Warn,
                    "info" => Level::Info,
                    _ => Level::Log,
                };
                let text = p["args"].as_array().map(|a| a.iter().map(describe).collect::<Vec<_>>().join(" ")).unwrap_or_default();
                on(Ev::Console(level, text));
            }
            "Runtime.exceptionThrown" => {
                let ex = &p["exceptionDetails"];
                let text = ex["exception"]["description"].as_str().or(ex["text"].as_str()).unwrap_or("Uncaught error").to_owned();
                on(Ev::Console(Level::Error, text));
            }
            "Log.entryAdded" => {
                let e = &p["entry"];
                let level = match e["level"].as_str().unwrap_or("info") {
                    "error" => Level::Error,
                    "warning" => Level::Warn,
                    _ => Level::Info,
                };
                on(Ev::Console(level, e["text"].as_str().unwrap_or_default().to_owned()));
            }
            "Runtime.executionContextsCleared" => {
                on(Ev::Navigated);
                self.call("DOM.getDocument", json!({ "depth": 3 }), Pending::Document);
            }
            "DOM.documentUpdated" => self.call("DOM.getDocument", json!({ "depth": 3 }), Pending::Document),
            "DOM.setChildNodes" => {
                let nodes = p["nodes"].as_array().map(|a| a.iter().map(parse_node).collect()).unwrap_or_default();
                on(Ev::Nodes { parent: p["parentId"].as_i64(), nodes });
            }
            "Network.requestWillBeSent" => on(Ev::Request {
                id: s("requestId"),
                method: p["request"]["method"].as_str().unwrap_or("GET").to_owned(),
                url: p["request"]["url"].as_str().unwrap_or_default().to_owned(),
                kind: s("type"),
            }),
            "Network.responseReceived" => on(Ev::Response {
                id: s("requestId"),
                status: p["response"]["status"].as_u64().unwrap_or(0) as u16,
                kind: s("type"),
            }),
            "Network.loadingFinished" => on(Ev::Done { id: s("requestId"), bytes: p["encodedDataLength"].as_f64().unwrap_or(0.0), error: None }),
            "Network.loadingFailed" => on(Ev::Done { id: s("requestId"), bytes: 0.0, error: Some(s("errorText")) }),
            _ => {}
        }
    }
}

/// A console value the way DevTools prints it: strings bare, the rest by their description.
fn describe(o: &Value) -> String {
    match o["type"].as_str() {
        Some("string") => o["value"].as_str().unwrap_or_default().to_owned(),
        Some("undefined") => "undefined".into(),
        _ => match o.get("value") {
            Some(v) if !v.is_null() || o["subtype"] == "null" => v.to_string(),
            _ => o["description"].as_str().unwrap_or_default().to_owned(),
        },
    }
}

fn parse_node(v: &Value) -> Node {
    let id = v["nodeId"].as_i64().unwrap_or(0);
    let kind = v["nodeType"].as_u64().unwrap_or(1);
    let children = v["children"].as_array().map(|a| a.iter().map(parse_node).filter(|n| !n.name.is_empty()).collect::<Vec<_>>());
    let child_count = v["childNodeCount"].as_u64().map_or_else(|| children.as_ref().map_or(0, Vec::len), |c| c as usize);
    let (name, detail) = match kind {
        1 => {
            let mut d = String::new();
            if let Some(a) = v["attributes"].as_array() {
                for pair in a.chunks(2) {
                    let (k, val) = (pair[0].as_str().unwrap_or_default(), pair.get(1).and_then(Value::as_str).unwrap_or_default());
                    if matches!(k, "id" | "class" | "name" | "type" | "href" | "src" | "rel") {
                        let val: String = val.chars().take(60).collect();
                        d.push_str(&format!(" {k}=\"{val}\""));
                    }
                }
            }
            (v["localName"].as_str().unwrap_or("?").to_owned(), d)
        }
        3 => {
            let t = v["nodeValue"].as_str().unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
            if t.is_empty() { (String::new(), String::new()) } else { ("#text".into(), t.chars().take(120).collect()) }
        }
        10 => (format!("<!DOCTYPE {}>", v["nodeName"].as_str().unwrap_or("html").to_lowercase()), String::new()),
        // The document itself and fragments: shown through their children.
        9 | 11 => ("#document".into(), String::new()),
        _ => (String::new(), String::new()),
    };
    Node { id, name, detail, child_count, children }
}

// ---------- panel ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Elements,
    Console,
    Network,
}

/// What a click or key in the panel asks of the browser.
pub enum PanelCmd {
    Close,
    Send(Cmd),
}

struct Req {
    method: String,
    url: String,
    kind: String,
    status: u16,
    bytes: f64,
    error: Option<String>,
    start: Instant,
    took: Option<Duration>,
}

struct DomNode {
    node: Node,
    kids: Option<Vec<i64>>,
    open: bool,
    depth: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Tab(View),
    Close,
    Clear,
    Prompt,
    Row(usize),
    Toggle(usize),
}

#[derive(Clone, Copy)]
struct Area {
    hit: Hit,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

pub struct Panel {
    engine: Option<Engine>,
    theme: Theme,
    dark: bool,
    s: f32,
    w: f32,
    h: f32,
    pub view: View,
    status: String,
    console: Vec<(Level, String)>,
    input: String,
    ed: Editor,
    history: Vec<String>,
    hist_at: Option<usize>,
    /// The prompt has the keyboard.
    pub typing: bool,
    reqs: Vec<Req>,
    req_index: HashMap<String, usize>,
    dom: HashMap<i64, DomNode>,
    roots: Vec<i64>,
    /// Elements rows as last laid out: node id per row.
    rows: Vec<i64>,
    scroll: [f32; 3],
    hover: Option<Hit>,
    highlighted: Option<i64>,
    areas: Vec<Area>,
    dirty: bool,
    pub frame: Pixmap,
}

const HEAD_H: f32 = 34.0;
const ROW_H: f32 = 20.0;
const PROMPT_H: f32 = 30.0;
const FONT: f32 = 12.0;
const MAX_LINES: usize = 1000;

impl Panel {
    pub fn new(engine: Option<Engine>, dark: bool, color: PersonaColor) -> Self {
        Self {
            engine,
            theme: Theme::for_persona(dark, color),
            dark,
            s: 1.0,
            w: 1.0,
            h: 1.0,
            view: View::Console,
            status: String::new(),
            console: Vec::new(),
            input: String::new(),
            ed: Editor::at_end(0),
            history: Vec::new(),
            hist_at: None,
            typing: false,
            reqs: Vec::new(),
            req_index: HashMap::new(),
            dom: HashMap::new(),
            roots: Vec::new(),
            rows: Vec::new(),
            scroll: [0.0; 3],
            hover: None,
            highlighted: None,
            areas: Vec::new(),
            dirty: true,
            frame: Pixmap::new(1, 1).expect("1x1 pixmap"),
        }
    }

    pub fn set_theme(&mut self, dark: bool, color: PersonaColor) {
        self.dark = dark;
        self.theme = Theme::for_persona(dark, color);
        self.dirty = true;
    }

    pub fn set_size(&mut self, scale: f32, w: f32, h: f32) {
        if (scale, w, h) != (self.s, self.w, self.h) {
            (self.s, self.w, self.h) = (scale, w.max(1.0), h.max(1.0));
            self.dirty = true;
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Starts over for another page.
    pub fn reset(&mut self, status: &str) {
        self.console.clear();
        self.reqs.clear();
        self.req_index.clear();
        self.dom.clear();
        self.roots.clear();
        self.scroll = [0.0; 3];
        self.highlighted = None;
        self.status = status.to_owned();
        self.dirty = true;
    }

    /// Frees the panel's picture while it is closed.
    pub fn release(&mut self) {
        self.frame = Pixmap::new(1, 1).expect("1x1 pixmap");
        self.dirty = true;
    }

    pub fn event(&mut self, ev: Ev) {
        self.dirty = true;
        match ev {
            Ev::Attached => self.status.clear(),
            Ev::Failed(why) => self.status = why,
            Ev::Navigated => {
                self.console.clear();
                self.reqs.clear();
                self.req_index.clear();
                self.dom.clear();
                self.roots.clear();
                self.highlighted = None;
            }
            Ev::Console(level, text) => {
                self.console.push((level, text));
                if self.console.len() > MAX_LINES {
                    self.console.drain(..self.console.len() - MAX_LINES);
                }
                // Follow new output.
                self.scroll[1] = f32::MAX;
            }
            Ev::Request { id, method, url, kind } => {
                if self.reqs.len() >= MAX_LINES {
                    self.reqs.clear();
                    self.req_index.clear();
                }
                self.req_index.insert(id, self.reqs.len());
                self.reqs.push(Req { method, url, kind, status: 0, bytes: 0.0, error: None, start: Instant::now(), took: None });
            }
            Ev::Response { id, status, kind } => {
                if let Some(r) = self.req_index.get(&id).and_then(|&i| self.reqs.get_mut(i)) {
                    r.status = status;
                    if !kind.is_empty() {
                        r.kind = kind;
                    }
                }
            }
            Ev::Done { id, bytes, error } => {
                if let Some(r) = self.req_index.get(&id).and_then(|&i| self.reqs.get_mut(i)) {
                    r.bytes = bytes;
                    r.error = error;
                    r.took = Some(r.start.elapsed());
                }
            }
            Ev::Nodes { parent: None, nodes } => {
                // A whole new document.
                self.dom.clear();
                self.roots = self.add_nodes(nodes, 0);
            }
            Ev::Nodes { parent: Some(p), nodes } => {
                let depth = self.dom.get(&p).map_or(0, |d| d.depth + 1);
                let ids = self.add_nodes(nodes, depth);
                if let Some(d) = self.dom.get_mut(&p) {
                    d.kids = Some(ids);
                    d.open = true;
                }
            }
        }
    }

    fn add_nodes(&mut self, nodes: Vec<Node>, depth: usize) -> Vec<i64> {
        let mut ids = Vec::with_capacity(nodes.len());
        for mut n in nodes {
            let kids = n.children.take().map(|c| self.add_nodes(c, depth + 1));
            // The page's <html> and <body> start open, like in other browsers' DevTools.
            let open = kids.is_some() && (n.name == "html" || n.name == "body");
            ids.push(n.id);
            self.dom.insert(n.id, DomNode { node: n, kids, open, depth });
        }
        ids
    }

    fn area(&mut self, hit: Hit, x: f32, y: f32, w: f32, h: f32) {
        self.areas.push(Area { hit, x, y, w, h });
    }

    fn hit_at(&self, x: f32, y: f32) -> Option<Hit> {
        self.areas.iter().rev().find(|a| x >= a.x && x < a.x + a.w && y >= a.y && y < a.y + a.h).map(|a| a.hit)
    }

    /// Re-draws into `frame` if anything changed. Returns true when it did.
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
        let bg = self.theme.toolbar;
        let Some(mut engine) = self.engine.take() else {
            self.frame.fill(tiny_skia::Color::from_rgba8(bg.0, bg.1, bg.2, 255));
            return true;
        };
        let frame = engine.frame(pw, ph, self.s, bg, |p| self.paint(p));
        anvil::copy_to_pixmap(&frame, &mut self.frame);
        self.engine = Some(engine);
        true
    }

    fn paint(&mut self, c: &mut Painter) {
        self.areas.clear();
        let th = self.theme;
        let (w, h) = (self.w, self.h);
        c.backdrop = th.toolbar;
        // Header: the three tabs, clear, close.
        c.rect(0.0, 0.0, w, HEAD_H, th.strip);
        c.hairline_h(0.0, HEAD_H - 1.0, w, th.border);
        c.hairline_v(0.0, 0.0, h, th.border);
        let mut x = 10.0;
        for (v, label) in [(View::Elements, "Elements"), (View::Console, "Console"), (View::Network, "Network")] {
            let tw = c.text_width(label, 12.5, Weight::SemiBold);
            let on = self.view == v;
            let hot = self.hover == Some(Hit::Tab(v));
            let col = if on { th.fg } else if hot { th.fg.with_alpha(0.85) } else { th.dim };
            let ty = (HEAD_H - c.line_height(12.5, Weight::SemiBold)) / 2.0;
            c.text(label, x + 8.0, ty, 12.5, Weight::SemiBold, col, 1.0e9);
            if on {
                c.rrect(x + 6.0, HEAD_H - 3.0, tw + 4.0, 2.0, 1.0, th.accent);
            }
            self.area(Hit::Tab(v), x, 0.0, tw + 16.0, HEAD_H);
            x += tw + 18.0;
        }
        c.backdrop = th.strip;
        let bx = w - 34.0;
        if self.hover == Some(Hit::Close) {
            c.rrect(bx + 3.0, 5.0, 24.0, 24.0, 6.0, th.btn_hover);
        }
        c.icon(Icon::Close, bx + 8.0, 10.0, 14.0, th.fg);
        self.area(Hit::Close, bx, 0.0, 30.0, HEAD_H);
        if self.view != View::Elements {
            let cx = bx - 30.0;
            if self.hover == Some(Hit::Clear) {
                c.rrect(cx + 3.0, 5.0, 24.0, 24.0, 6.0, th.btn_hover);
            }
            c.icon(Icon::Trash, cx + 8.0, 10.0, 14.0, th.dim);
            self.area(Hit::Clear, cx, 0.0, 30.0, HEAD_H);
        }
        c.backdrop = th.toolbar;
        if !self.status.is_empty() {
            c.text(&self.status, 14.0, HEAD_H + 12.0, FONT, Weight::Regular, th.dim, w - 14.0);
            return;
        }
        match self.view {
            View::Console => self.paint_console(c, &th),
            View::Network => self.paint_network(c, &th),
            View::Elements => self.paint_elements(c, &th),
        }
    }

    /// Clamps a tab's scroll to its content and returns it.
    fn scrolled(&mut self, k: usize, content: f32, visible: f32) -> f32 {
        let max = (content - visible).max(0.0);
        self.scroll[k] = self.scroll[k].clamp(0.0, max);
        self.scroll[k]
    }

    fn paint_console(&mut self, c: &mut Painter, th: &Theme) {
        let (w, h) = (self.w, self.h);
        let body_h = h - HEAD_H - PROMPT_H;
        let lines: Vec<(Level, String)> = self.console.iter().flat_map(|(l, t)| t.lines().take(40).map(move |s| (*l, s.to_owned()))).collect();
        let n = lines.len();
        let top = self.scrolled(1, n as f32 * ROW_H, body_h);
        let first = (top / ROW_H) as usize;
        let mut y = HEAD_H + first as f32 * ROW_H - top;
        for (level, text) in lines.into_iter().skip(first) {
            if y > HEAD_H + body_h {
                break;
            }
            let (bg, fg, mark) = match level {
                Level::Error => (Rgba(229, 72, 77, 255).with_alpha(0.12), Rgba(240, 96, 100, 255), "✕"),
                Level::Warn => (Rgba(245, 180, 40, 255).with_alpha(0.12), if self.dark { Rgba(250, 200, 90, 255) } else { Rgba(160, 100, 0, 255) }, "!"),
                Level::Input => (Rgba(0, 0, 0, 0), th.accent, "›"),
                Level::Result => (Rgba(0, 0, 0, 0), th.dim, "‹"),
                Level::Info | Level::Log => (Rgba(0, 0, 0, 0), th.fg, " "),
            };
            c.rect(1.0, y, w, ROW_H, bg);
            c.text(mark, 10.0, y + 3.0, FONT, Weight::Mono, fg, 24.0);
            c.text(&text, 26.0, y + 3.0, FONT, Weight::Mono, fg, w - 8.0);
            c.hairline_h(1.0, y + ROW_H - 1.0, w, th.border.with_alpha(0.5));
            y += ROW_H;
        }
        // The prompt.
        let py = h - PROMPT_H;
        c.rect(1.0, py, w, PROMPT_H, if self.typing { th.field } else { th.toolbar });
        c.hairline_h(0.0, py, w, th.border);
        c.text("›", 10.0, py + 7.0, 13.0, Weight::Mono, th.accent, 24.0);
        if self.input.is_empty() && !self.typing {
            c.text("Type JavaScript and press Enter", 26.0, py + 8.0, FONT, Weight::Regular, th.dim, w - 8.0);
        } else {
            c.text_cut(&self.input, 26.0, py + 7.0, FONT, Weight::Mono, th.fg, (26.0, w - 8.0));
        }
        if self.typing {
            let before: String = self.input.chars().take(self.ed.caret).collect();
            let cx = 26.0 + c.text_width(&before, FONT, Weight::Mono);
            c.rect(cx, py + 7.0, 1.5, 16.0, th.accent);
        }
        self.area(Hit::Prompt, 0.0, py, w, PROMPT_H);
    }

    fn paint_network(&mut self, c: &mut Painter, th: &Theme) {
        let (w, h) = (self.w, self.h);
        // Columns: name, status, type, size, time.
        let cols = [w - 300.0, 56.0, 76.0, 76.0, 70.0];
        let head_y = HEAD_H;
        c.rect(1.0, head_y, w, ROW_H + 2.0, th.strip.mix(th.toolbar, 0.5));
        let mut x = 10.0;
        for (label, cw) in ["Name", "Status", "Type", "Size", "Time"].iter().zip(cols) {
            c.text(label, x, head_y + 4.0, 11.5, Weight::SemiBold, th.dim, x + cw - 8.0);
            x += cw;
        }
        let body_y = head_y + ROW_H + 2.0;
        let body_h = h - body_y;
        let n = self.reqs.len();
        let top = self.scrolled(2, n as f32 * ROW_H, body_h);
        let first = (top / ROW_H) as usize;
        let mut y = body_y + first as f32 * ROW_H - top;
        let mut rows = Vec::new();
        for (i, r) in self.reqs.iter().enumerate().skip(first) {
            if y > h {
                break;
            }
            let failed = r.error.is_some() || r.status >= 400;
            let fg = if failed { Rgba(240, 96, 100, 255) } else { th.fg };
            if self.hover == Some(Hit::Row(i)) {
                c.rect(1.0, y, w, ROW_H, th.btn_hover);
            }
            let name = short_name(&r.url);
            let status = match (&r.error, r.status) {
                (Some(_), _) => "failed".to_owned(),
                (None, 0) => "…".to_owned(),
                (None, s) => s.to_string(),
            };
            let size = if r.bytes <= 0.0 { "–".to_owned() } else { human_bytes(r.bytes) };
            let took = r.took.map_or("…".to_owned(), |d| format!("{} ms", d.as_millis()));
            let kind = r.kind.to_lowercase();
            let cells = [format!("{} {name}", r.method), status, kind, size, took];
            let mut x = 10.0;
            for (k, (cell, cw)) in cells.iter().zip(cols).enumerate() {
                c.text(cell, x, y + 3.0, FONT, if k == 0 { Weight::Mono } else { Weight::Regular }, if k == 0 { fg } else { th.dim }, x + cw - 8.0);
                x += cw;
            }
            c.hairline_h(1.0, y + ROW_H - 1.0, w, th.border.with_alpha(0.5));
            rows.push((i, y));
            y += ROW_H;
        }
        for (i, y) in rows {
            self.area(Hit::Row(i), 0.0, y, w, ROW_H);
        }
        let _ = th;
    }

    fn paint_elements(&mut self, c: &mut Painter, th: &Theme) {
        let (w, h) = (self.w, self.h);
        // Flatten the open part of the tree into rows.
        let mut rows = Vec::new();
        let mut stack: Vec<i64> = self.roots.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            let Some(d) = self.dom.get(&id) else { continue };
            rows.push(id);
            if d.open
                && let Some(k) = &d.kids
            {
                stack.extend(k.iter().rev());
            }
        }
        self.rows = rows.clone();
        let body_y = HEAD_H + 4.0;
        let body_h = h - body_y;
        let top = self.scrolled(0, rows.len() as f32 * ROW_H, body_h);
        let first = (top / ROW_H) as usize;
        let mut y = body_y + first as f32 * ROW_H - top;
        let tag = th.accent;
        let attr = if self.dark { Rgba(150, 190, 255, 255) } else { Rgba(30, 90, 190, 255) };
        for (i, id) in rows.iter().enumerate().skip(first) {
            if y > h {
                break;
            }
            let Some(d) = self.dom.get(id) else { continue };
            if self.hover == Some(Hit::Row(i)) || self.hover == Some(Hit::Toggle(i)) {
                c.rect(1.0, y, w, ROW_H, th.btn_hover);
            }
            let x = 10.0 + d.depth as f32 * 14.0;
            let n = &d.node;
            if n.child_count > 0 && n.name != "#text" {
                let ang = if d.open { std::f32::consts::FRAC_PI_2 } else { 0.0 };
                c.icon_rot(Icon::ChevronRight, x, y + 4.0, 12.0, th.dim, ang);
                self.areas.push(Area { hit: Hit::Toggle(i), x: x - 4.0, y, w: 20.0, h: ROW_H });
            }
            let tx = x + 16.0;
            match n.name.as_str() {
                "#text" => {
                    c.text(&format!("\"{}\"", n.detail), tx, y + 3.0, FONT, Weight::Mono, th.fg, w - 8.0);
                }
                name if name.starts_with('<') || name.starts_with('#') => {
                    c.text(name, tx, y + 3.0, FONT, Weight::Mono, th.dim, w - 8.0);
                }
                name => {
                    let open = format!("<{name}");
                    let ow = c.text(&open, tx, y + 3.0, FONT, Weight::Mono, tag, w - 8.0);
                    let aw = c.text(&n.detail, tx + ow, y + 3.0, FONT, Weight::Mono, attr, w - 8.0);
                    let close = if d.open || n.child_count == 0 { ">".to_owned() } else { ">…</".to_owned() + name + ">" };
                    c.text(&close, tx + ow + aw, y + 3.0, FONT, Weight::Mono, tag, w - 8.0);
                }
            }
            self.areas.push(Area { hit: Hit::Row(i), x: 0.0, y, w, h: ROW_H });
            y += ROW_H;
        }
    }

    /// Pointer moved (panel coordinates). Returns the cursor and maybe a command (highlight).
    pub fn mouse_move(&mut self, x: f32, y: f32) -> (CursorIcon, Option<PanelCmd>) {
        let hit = self.hit_at(x, y);
        if hit != self.hover {
            self.hover = hit;
            self.dirty = true;
        }
        // Hovering an element outlines it on the page.
        let node = match (self.view, hit) {
            (View::Elements, Some(Hit::Row(i) | Hit::Toggle(i))) => self.rows.get(i).copied(),
            _ => None,
        };
        let cmd = (node != self.highlighted).then(|| {
            self.highlighted = node;
            PanelCmd::Send(Cmd::Highlight(node))
        });
        let icon = match hit {
            Some(Hit::Prompt) => CursorIcon::Text,
            Some(Hit::Row(_)) | None => CursorIcon::Default,
            Some(_) => CursorIcon::Hand,
        };
        (icon, cmd)
    }

    pub fn mouse_left(&mut self) -> Option<PanelCmd> {
        if self.hover.take().is_some() {
            self.dirty = true;
        }
        self.highlighted.take().map(|_| PanelCmd::Send(Cmd::Highlight(None)))
    }

    pub fn click(&mut self, x: f32, y: f32) -> Option<PanelCmd> {
        let hit = self.hit_at(x, y);
        self.dirty = true;
        self.typing = hit == Some(Hit::Prompt);
        match hit? {
            Hit::Tab(v) => {
                self.view = v;
                None
            }
            Hit::Close => Some(PanelCmd::Close),
            Hit::Clear => {
                match self.view {
                    View::Console => self.console.clear(),
                    View::Network => {
                        self.reqs.clear();
                        self.req_index.clear();
                    }
                    View::Elements => {}
                }
                None
            }
            Hit::Toggle(i) => {
                let id = *self.rows.get(i)?;
                let d = self.dom.get_mut(&id)?;
                if d.open {
                    d.open = false;
                    None
                } else if d.kids.is_some() {
                    d.open = true;
                    None
                } else {
                    Some(PanelCmd::Send(Cmd::Expand(id)))
                }
            }
            Hit::Prompt | Hit::Row(_) => None,
        }
    }

    pub fn wheel(&mut self, dy: f32) {
        let k = match self.view {
            View::Elements => 0,
            View::Console => 1,
            View::Network => 2,
        };
        if self.scroll[k] == f32::MAX {
            // Following the end: start from there.
            self.scroll[k] = 1.0e7;
        }
        self.scroll[k] = (self.scroll[k] - dy).max(0.0);
        self.dirty = true;
    }

    /// A key while the prompt has the keyboard. Returns a command (evaluate) when Enter is hit.
    pub fn key(&mut self, e: &tao::event::KeyEvent, mods: ModifiersState) -> Option<PanelCmd> {
        if e.state != ElementState::Pressed || !self.typing {
            return None;
        }
        self.dirty = true;
        let (word, shift) = (mods.control_key(), mods.shift_key());
        match &e.logical_key {
            Key::Enter => {
                let line = std::mem::take(&mut self.input);
                self.ed = Editor::at_end(0);
                self.hist_at = None;
                if line.trim().is_empty() {
                    return None;
                }
                self.history.push(line.clone());
                self.console.push((Level::Input, line.clone()));
                self.scroll[1] = f32::MAX;
                return Some(PanelCmd::Send(Cmd::Eval(line)));
            }
            Key::Escape => self.typing = false,
            Key::Backspace => self.ed.backspace(&mut self.input, word),
            Key::Delete => self.ed.delete_forward(&mut self.input, word),
            Key::ArrowLeft => self.ed.left(&self.input, shift, word),
            Key::ArrowRight => self.ed.right(&self.input, shift, word),
            Key::Home => self.ed.home(shift),
            Key::End => self.ed.end(&self.input, shift),
            Key::ArrowUp | Key::ArrowDown if !self.history.is_empty() => {
                let up = e.logical_key == Key::ArrowUp;
                let n = self.history.len();
                let at = match (self.hist_at, up) {
                    (None, true) => Some(n - 1),
                    (Some(i), true) => Some(i.saturating_sub(1)),
                    (Some(i), false) if i + 1 < n => Some(i + 1),
                    _ => None,
                };
                self.hist_at = at;
                self.input = at.map(|i| self.history[i].clone()).unwrap_or_default();
                self.ed = Editor::at_end(self.input.chars().count());
            }
            _ if word && matches!(&e.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("a")) => self.ed.select_all(self.input.chars().count()),
            _ if word && matches!(&e.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("l")) => self.console.clear(),
            _ if !word && !mods.alt_key() => {
                if let Some(t) = e.text.filter(|t| !t.chars().all(char::is_control)) {
                    self.ed.insert(&mut self.input, t, 4000, &InputKind::Text);
                }
            }
            _ => {}
        }
        None
    }

    /// Text from the input method (non-Latin keyboards) while the prompt has the keyboard.
    pub fn ime(&mut self, text: &str) {
        if self.typing {
            self.ed.insert(&mut self.input, text, 4000, &InputKind::Text);
            self.dirty = true;
        }
    }
}

/// A request's name as DevTools lists it: the last path segment (or the host).
fn short_name(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let path = rest.split(['?', '#']).next().unwrap_or(rest);
    let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or(path);
    if last.is_empty() { rest.to_owned() } else { last.chars().take(80).collect() }
}

fn human_bytes(b: f64) -> String {
    if b < 1024.0 {
        format!("{b:.0} B")
    } else if b < 1024.0 * 1024.0 {
        format!("{:.1} kB", b / 1024.0)
    } else {
        format!("{:.1} MB", b / 1024.0 / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_values_print_like_devtools() {
        assert_eq!(describe(&json!({ "type": "string", "value": "hi" })), "hi");
        assert_eq!(describe(&json!({ "type": "number", "value": 3, "description": "3" })), "3");
        assert_eq!(describe(&json!({ "type": "undefined" })), "undefined");
        assert_eq!(describe(&json!({ "type": "object", "subtype": "null", "value": null })), "null");
        assert_eq!(describe(&json!({ "type": "object", "className": "Object", "description": "Object" })), "Object");
    }

    #[test]
    fn dom_nodes_parse_with_children() {
        let doc = json!({ "nodeId": 1, "nodeType": 9, "children": [
            { "nodeId": 2, "nodeType": 10, "nodeName": "html" },
            { "nodeId": 3, "nodeType": 1, "localName": "html", "childNodeCount": 2, "attributes": ["lang", "en"], "children": [
                { "nodeId": 4, "nodeType": 1, "localName": "body", "childNodeCount": 1, "attributes": ["class", "page", "data-x", "y"] },
                { "nodeId": 5, "nodeType": 3, "nodeValue": "   " }
            ]}
        ]});
        let n = parse_node(&doc);
        let kids = n.children.unwrap();
        assert_eq!(kids[0].name, "<!DOCTYPE html>");
        assert_eq!(kids[1].name, "html");
        let html_kids = kids[1].children.as_ref().unwrap();
        // Whitespace-only text nodes are dropped; only the interesting attributes are kept.
        assert_eq!(html_kids.len(), 1);
        assert_eq!(html_kids[0].detail, " class=\"page\"");
    }

    #[test]
    fn panel_tracks_requests_and_prompt() {
        let mut p = Panel::new(None, true, PersonaColor::Orange);
        p.event(Ev::Request { id: "1".into(), method: "GET".into(), url: "https://example.com/app.js?v=2".into(), kind: "Script".into() });
        p.event(Ev::Response { id: "1".into(), status: 200, kind: String::new() });
        p.event(Ev::Done { id: "1".into(), bytes: 2048.0, error: None });
        assert_eq!(p.reqs[0].status, 200);
        assert_eq!(short_name(&p.reqs[0].url), "app.js");
        assert_eq!(human_bytes(p.reqs[0].bytes), "2.0 kB");
        p.event(Ev::Nodes { parent: None, nodes: vec![Node { id: 3, name: "html".into(), child_count: 1, ..Default::default() }] });
        assert_eq!(p.roots, vec![3]);
        assert!(p.dom.contains_key(&3));
        p.event(Ev::Nodes { parent: Some(3), nodes: vec![Node { id: 4, name: "body".into(), ..Default::default() }] });
        assert_eq!(p.dom[&4].depth, 1);
    }
}
