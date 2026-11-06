//! What an app needs from the OS beyond a window: the clipboard, and on Android the soft
//! keyboard and the system bars' insets. Desktop builds get no-op stand-ins where the OS has no
//! such thing, so app code needs no `cfg`.

mod clipboard;
pub mod permission;
pub mod soft_input;

pub use clipboard::Clipboard;
