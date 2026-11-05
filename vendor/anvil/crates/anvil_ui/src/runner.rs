//! Runs an app on its own UI thread, apart from the window's event thread.
//!
//! The event thread (winit's, which the OS requires to be the main thread) only creates the
//! window and forwards its events. The app builds, renders and waits for vsync on the UI
//! thread. So:
//!   * the OS can block the event thread (Windows does while the title bar is held, moved or
//!     resized) without stopping animations, sync or redraws;
//!   * waiting for the display never delays the next input event;
//!   * frames run at the display's own rate: an app asks for the next frame and the swapchain
//!     paces it, at 60, 144 or 280 Hz alike;
//!   * a resize waits (at most `RESIZE_WAIT`) for a frame at the new size, so the window's edge
//!     and its content move together instead of Windows stretching the old frame.
//!
//! With nothing to draw and no timer due, the UI thread blocks on its channel and uses no CPU.
//!
//! ```ignore
//! let event_loop = EventLoopBuilder::with_user_event().build()?;
//! runner::run(event_loop, |target| WindowBuilder::new().build(target).unwrap(), MyApp::new);
//! ```

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::time::{Duration, Instant};

use winit::event::{Event, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop, EventLoopWindowTarget};
use winit::window::Window;

/// How long the event thread waits, on a resize, for the UI thread to draw at the new size. A
/// frame takes about a millisecond; the cap only matters if the UI thread is stuck.
const RESIZE_WAIT: Duration = Duration::from_millis(100);

/// The event loop's user event: the UI thread asking the event thread to stop.
#[derive(Debug)]
pub struct Exit;

/// What the UI thread receives.
enum Msg<E> {
    Resumed(&'static Window),
    /// Android is about to destroy the surface. The UI thread answers once it has let go.
    Suspended(SyncSender<()>),
    Window(WindowEvent),
    /// The window was resized. The UI thread answers once a frame at the new size is drawn.
    Resized(WindowEvent, SyncSender<()>),
    User(E),
    /// The OS asked for the window to be drawn.
    Redraw,
    Exit,
}

/// Sends the app's own events (from worker threads, sockets, timers) to its UI thread.
pub struct Proxy<E> {
    tx: Sender<Msg<E>>,
}

impl<E> Clone for Proxy<E> {
    fn clone(&self) -> Self {
        Self { tx: self.tx.clone() }
    }
}

impl<E> Proxy<E> {
    /// Queues `event` for the app. Returns false once the app has stopped.
    pub fn send(&self, event: E) -> bool {
        self.tx.send(Msg::User(event)).is_ok()
    }
}

/// What the app asks of the runner while handling events or drawing.
#[derive(Default)]
pub struct Cx {
    redraw: bool,
    wake: Option<Instant>,
    exit: bool,
}

impl Cx {
    /// Draws a frame as soon as the events waiting now are handled.
    pub fn request_redraw(&mut self) {
        self.redraw = true;
    }

    /// Wakes the app at `at` even if no event arrives. Several requests keep the soonest.
    pub fn wake_at(&mut self, at: Instant) {
        self.wake = Some(self.wake.map_or(at, |w| w.min(at)));
    }

    /// Stops the app: `App::exiting` runs, then the window closes.
    pub fn exit(&mut self) {
        self.exit = true;
    }
}

/// An app the runner drives. It is created on the UI thread and lives there.
pub trait App {
    type Event: Send + 'static;

    /// The window exists, or (Android) its surface is back after a suspend. Make or remake the
    /// renderer's surface here.
    fn resumed(&mut self, window: &'static Window);

    /// The surface is going away (Android). Stop drawing until `resumed`.
    fn suspended(&mut self);

    fn window_event(&mut self, event: WindowEvent, cx: &mut Cx);

    fn user_event(&mut self, event: Self::Event, cx: &mut Cx);

    /// Runs after each batch of events and after each frame: ask for a redraw or a wake here.
    fn about_to_wait(&mut self, cx: &mut Cx);

    /// Builds and draws one frame. With vsync, presenting waits for the display, which paces
    /// animation at the display's rate.
    fn frame(&mut self, cx: &mut Cx);

    /// The app is stopping: save what needs saving.
    fn exiting(&mut self);
}

/// Runs `make_app`'s app on a new UI thread until it exits or its window closes. `make_window`
/// creates the window on the event thread when the app first starts.
pub fn run<A, W, M>(event_loop: EventLoop<Exit>, make_window: W, make_app: M)
where
    A: App,
    W: Fn(&EventLoopWindowTarget<Exit>) -> Window + 'static,
    M: FnOnce(Proxy<A::Event>) -> A + Send + 'static,
{
    crate::crash::install();
    let (tx, rx) = mpsc::channel::<Msg<A::Event>>();
    let proxy = Proxy { tx: tx.clone() };
    let exit_proxy = event_loop.create_proxy();
    let ui_thread = std::thread::Builder::new().name("anvil-ui".into()).spawn(move || {
        ui_loop(make_app(proxy), rx, move || {
            let _ = exit_proxy.send_event(Exit);
        })
    });
    let ui_thread = match ui_thread {
        Ok(t) => t,
        Err(e) => {
            eprintln!("anvil: failed to start the UI thread: {e}");
            return;
        }
    };
    let mut ui_thread = Some(ui_thread);
    let mut window: Option<&'static Window> = None;

    if let Err(e) = event_loop
        .run(move |event, target| {
            target.set_control_flow(ControlFlow::Wait);
            match event {
                Event::Resumed => {
                    let w = *window.get_or_insert_with(|| Box::leak(Box::new(make_window(target))));
                    let _ = tx.send(Msg::Resumed(w));
                }
                Event::Suspended => {
                    // The surface must not be used after this returns, so wait for the UI thread.
                    let (ack_tx, ack_rx) = mpsc::sync_channel(0);
                    if tx.send(Msg::Suspended(ack_tx)).is_ok() {
                        let _ = ack_rx.recv();
                    }
                }
                Event::WindowEvent { window_id, event } if window.is_some_and(|w| w.id() == window_id) => {
                    match event {
                        WindowEvent::RedrawRequested => {
                            let _ = tx.send(Msg::Redraw);
                        }
                        WindowEvent::Resized(_) => {
                            // Hold the window's thread until the content matches its new size.
                            let (ack_tx, ack_rx) = mpsc::sync_channel(1);
                            if tx.send(Msg::Resized(event, ack_tx)).is_ok() {
                                let _ = ack_rx.recv_timeout(RESIZE_WAIT);
                            }
                        }
                        event => {
                            let _ = tx.send(Msg::Window(event));
                        }
                    }
                }
                Event::UserEvent(Exit) => target.exit(),
                Event::LoopExiting => {
                    let _ = tx.send(Msg::Exit);
                    if let Some(t) = ui_thread.take() {
                        let _ = t.join();
                    }
                }
                _ => {}
            }
        })
    {
        eprintln!("anvil: event loop terminated with an error: {e}");
    }
}

/// Runs `on_exit` when the UI thread ends, however it ends (a panic included), so the event
/// thread stops too.
struct OnDrop<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Drop for OnDrop<F> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}

fn ui_loop<A: App>(mut app: A, rx: Receiver<Msg<A::Event>>, on_exit: impl FnOnce()) {
    let _exit = OnDrop(Some(on_exit));
    let mut cx = Cx::default();
    // Resizes waiting for a frame at their size.
    let mut resizes: Vec<SyncSender<()>> = Vec::new();
    // Handles one message. Returns false when the app must stop.
    let handle = |app: &mut A, cx: &mut Cx, resizes: &mut Vec<SyncSender<()>>, msg: Msg<A::Event>| -> bool {
        match msg {
            Msg::Resumed(w) => {
                app.resumed(w);
                cx.redraw = true;
            }
            Msg::Suspended(ack) => {
                app.suspended();
                let _ = ack.send(());
            }
            Msg::Window(event) => app.window_event(event, cx),
            Msg::Resized(event, ack) => {
                app.window_event(event, cx);
                cx.redraw = true;
                resizes.push(ack);
            }
            Msg::User(event) => app.user_event(event, cx),
            Msg::Redraw => cx.redraw = true,
            Msg::Exit => return false,
        }
        true
    };
    'run: loop {
        cx.wake = None;
        app.about_to_wait(&mut cx);
        if cx.exit {
            break;
        }
        if std::mem::take(&mut cx.redraw) {
            app.frame(&mut cx);
            for ack in resizes.drain(..) {
                let _ = ack.send(());
            }
        } else {
            // Nothing to draw after all: let a waiting resize go.
            for ack in resizes.drain(..) {
                let _ = ack.send(());
            }
            // Nothing to draw: sleep until an event or the wake the app asked for.
            let first = match cx.wake {
                Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                    Ok(msg) => msg,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                },
                None => match rx.recv() {
                    Ok(msg) => msg,
                    Err(_) => break,
                },
            };
            if !handle(&mut app, &mut cx, &mut resizes, first) {
                break;
            }
        }
        // Everything that arrived meanwhile, before the next frame.
        while let Ok(msg) = rx.try_recv() {
            if !handle(&mut app, &mut cx, &mut resizes, msg) {
                break 'run;
            }
        }
    }
    app.exiting();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// Animates for `n` frames after an event `n`; the event 99 exits.
    #[derive(Default)]
    struct Probe {
        animate: u32,
        wake: Option<Instant>,
        exited: Arc<AtomicBool>,
    }

    impl App for Probe {
        type Event = u32;
        fn resumed(&mut self, _: &'static Window) {}
        fn suspended(&mut self) {}
        fn window_event(&mut self, _: WindowEvent, _: &mut Cx) {}
        fn user_event(&mut self, n: u32, cx: &mut Cx) {
            self.animate = n;
            if n == 99 {
                cx.exit();
            }
            cx.request_redraw();
        }
        fn about_to_wait(&mut self, cx: &mut Cx) {
            if self.animate > 0 {
                cx.request_redraw();
            }
            if let Some(at) = self.wake {
                cx.wake_at(at);
            }
        }
        fn frame(&mut self, _: &mut Cx) {
            self.animate = self.animate.saturating_sub(1);
        }
        fn exiting(&mut self) {
            self.exited.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn events_draw_animations_keep_drawing_and_exit_stops_both_threads() {
        let exited = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(std::sync::atomic::AtomicU32::new(0));
        struct Counting(Probe, Arc<std::sync::atomic::AtomicU32>);
        impl App for Counting {
            type Event = u32;
            fn resumed(&mut self, w: &'static Window) { self.0.resumed(w) }
            fn suspended(&mut self) { self.0.suspended() }
            fn window_event(&mut self, e: WindowEvent, cx: &mut Cx) { self.0.window_event(e, cx) }
            fn user_event(&mut self, n: u32, cx: &mut Cx) { self.0.user_event(n, cx) }
            fn about_to_wait(&mut self, cx: &mut Cx) { self.0.about_to_wait(cx) }
            fn frame(&mut self, cx: &mut Cx) {
                self.0.frame(cx);
                self.1.fetch_add(1, Ordering::SeqCst);
            }
            fn exiting(&mut self) { self.0.exiting() }
        }
        let (tx, rx) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = stopped.clone();
        let app = Counting(Probe { exited: exited.clone(), ..Default::default() }, frames.clone());
        let ui = std::thread::spawn(move || ui_loop(app, rx, move || flag.store(true, Ordering::SeqCst)));
        let proxy = Proxy { tx };

        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(frames.load(Ordering::SeqCst), 0, "idle: no frames");
        assert!(proxy.send(5));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(frames.load(Ordering::SeqCst), 5, "an event's frame, then it animates until it settles");
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(frames.load(Ordering::SeqCst), 5, "settled: asleep again");
        assert!(proxy.send(99));
        ui.join().unwrap();
        assert!(exited.load(Ordering::SeqCst), "exiting ran");
        assert!(stopped.load(Ordering::SeqCst), "the event thread was told to stop");
        assert!(!proxy.send(1), "nobody is listening");
    }

    #[test]
    fn a_wake_fires_without_events() {
        let (tx, rx) = mpsc::channel::<Msg<u32>>();
        let frames = Arc::new(std::sync::atomic::AtomicU32::new(0));
        struct Timer(Instant, Arc<std::sync::atomic::AtomicU32>, bool);
        impl App for Timer {
            type Event = u32;
            fn resumed(&mut self, _: &'static Window) {}
            fn suspended(&mut self) {}
            fn window_event(&mut self, _: WindowEvent, _: &mut Cx) {}
            fn user_event(&mut self, _: u32, cx: &mut Cx) { cx.exit() }
            fn about_to_wait(&mut self, cx: &mut Cx) {
                if !self.2 {
                    if Instant::now() >= self.0 {
                        self.2 = true;
                        cx.request_redraw();
                    } else {
                        cx.wake_at(self.0);
                    }
                }
            }
            fn frame(&mut self, _: &mut Cx) { self.1.fetch_add(1, Ordering::SeqCst); }
            fn exiting(&mut self) {}
        }
        // Generous margins, and a poll rather than a fixed sleep, so a busy machine (the whole
        // workspace testing in parallel) cannot make it flaky.
        let due = Instant::now() + Duration::from_millis(200);
        let app = Timer(due, frames.clone(), false);
        let ui = std::thread::spawn(move || ui_loop(app, rx, || {}));
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(frames.load(Ordering::SeqCst), 0, "not before the timer");
        let give_up = Instant::now() + Duration::from_secs(5);
        while frames.load(Ordering::SeqCst) == 0 && Instant::now() < give_up {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(frames.load(Ordering::SeqCst), 1, "the timer's frame");
        tx.send(Msg::User(0)).unwrap();
        ui.join().unwrap();
    }

    #[test]
    fn a_resize_is_released_once_a_frame_at_the_new_size_is_drawn() {
        let (tx, rx) = mpsc::channel::<Msg<u32>>();
        let frames = Arc::new(std::sync::atomic::AtomicU32::new(0));
        struct Draws(Arc<std::sync::atomic::AtomicU32>);
        impl App for Draws {
            type Event = u32;
            fn resumed(&mut self, _: &'static Window) {}
            fn suspended(&mut self) {}
            fn window_event(&mut self, _: WindowEvent, _: &mut Cx) {}
            fn user_event(&mut self, _: u32, cx: &mut Cx) { cx.exit() }
            fn about_to_wait(&mut self, _: &mut Cx) {}
            fn frame(&mut self, _: &mut Cx) {
                // A frame that takes a while: the resize must wait for it.
                std::thread::sleep(Duration::from_millis(20));
                self.0.fetch_add(1, Ordering::SeqCst);
            }
            fn exiting(&mut self) {}
        }
        let ui = std::thread::spawn({
            let frames = frames.clone();
            move || ui_loop(Draws(frames), rx, || {})
        });
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        let size = winit::dpi::PhysicalSize::new(800, 600);
        let start = Instant::now();
        tx.send(Msg::Resized(WindowEvent::Resized(size), ack_tx)).unwrap();
        ack_rx.recv_timeout(RESIZE_WAIT).expect("released before the cap");
        assert_eq!(frames.load(Ordering::SeqCst), 1, "after the frame, not before");
        assert!(start.elapsed() >= Duration::from_millis(20));
        tx.send(Msg::User(0)).unwrap();
        ui.join().unwrap();
    }
}
