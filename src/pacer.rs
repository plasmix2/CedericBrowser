//! Frame pacing for the native UI. Windows' default timer only ticks every ~15.6ms, so asking the
//! event loop to wake "in 16ms" really waits ~31ms and animations run at ~30fps. This thread uses
//! Rust's high-resolution sleep instead and wakes the UI every frame while something animates;
//! when nothing does, it is parked and costs nothing.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::Thread,
    time::Duration,
};

/// ~60 frames per second. Each chrome frame is drawn on the GPU and read back, so more than this
/// only starves the event loop (clicks and hover feel laggy).
pub const FRAME: Duration = Duration::from_micros(16_667);

pub struct Pacer {
    running: Arc<AtomicBool>,
    thread: Thread,
}

impl Pacer {
    pub fn spawn(on_frame: impl Fn() + Send + 'static) -> Self {
        let running = Arc::new(AtomicBool::new(false));
        let flag = running.clone();
        let handle = std::thread::spawn(move || {
            loop {
                if flag.load(Ordering::Acquire) {
                    std::thread::sleep(FRAME);
                    on_frame();
                } else {
                    std::thread::park();
                }
            }
        });
        Self { running, thread: handle.thread().clone() }
    }

    /// Starts or stops the frame ticks.
    pub fn set(&self, on: bool) {
        if self.running.swap(on, Ordering::AcqRel) != on && on {
            self.thread.unpark();
        }
    }
}
