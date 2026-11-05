//! Icon drawings from Tabler Icons (https://tabler.io/icons, MIT, (c) Paweł Kuna; see
//! `src/ui/TABLER-LICENSE.txt`), outline set, 24x24 grid, 2px stroke. Only the `d` attribute of each
//! path is kept; `anvil.rs` parses it into polylines for ANVL's GPU strokes.

use crate::gfx::Icon;

/// The path data of `icon`, one string per `<path>`.
pub fn paths(icon: Icon) -> &'static [&'static str] {
    match icon {
        Icon::Back => &[
            "M5 12l14 0",
            "M5 12l6 6",
            "M5 12l6 -6",
        ],
        Icon::Forward => &[
            "M5 12l14 0",
            "M13 18l6 -6",
            "M13 6l6 6",
        ],
        Icon::Reload => &[
            "M19.933 13.041a8 8 0 1 1 -9.925 -8.788c3.899 -1 7.935 1.007 9.425 4.747",
            "M20 4v5h-5",
        ],
        Icon::Stop => &[
            "M18 6l-12 12",
            "M6 6l12 12",
        ],
        Icon::Close => &[
            "M18 6l-12 12",
            "M6 6l12 12",
        ],
        Icon::Plus => &[
            "M12 5l0 14",
            "M5 12l14 0",
        ],
        Icon::Lock => &[
            "M5 13a2 2 0 0 1 2 -2h10a2 2 0 0 1 2 2v6a2 2 0 0 1 -2 2h-10a2 2 0 0 1 -2 -2v-6z",
            "M11 16a1 1 0 1 0 2 0a1 1 0 0 0 -2 0",
            "M8 11v-4a4 4 0 1 1 8 0v4",
        ],
        Icon::Info => &[
            "M3 12a9 9 0 1 0 18 0a9 9 0 0 0 -18 0",
            "M12 9h.01",
            "M11 12h1v4h1",
        ],
        Icon::Search => &[
            "M10 10m-7 0a7 7 0 1 0 14 0a7 7 0 1 0 -14 0",
            "M21 21l-6 -6",
        ],
        Icon::Globe => &[
            "M3 12a9 9 0 1 0 18 0a9 9 0 0 0 -18 0",
            "M3.6 9h16.8",
            "M3.6 15h16.8",
            "M11.5 3a17 17 0 0 0 0 18",
            "M12.5 3a17 17 0 0 1 0 18",
        ],
        Icon::Star => &[
            "M12 17.75l-6.172 3.245l1.179 -6.873l-5 -4.867l6.9 -1l3.086 -6.253l3.086 6.253l6.9 1l-5 4.867l1.179 6.873z",
        ],
        Icon::StarFilled => &[
            "M12 17.75l-6.172 3.245l1.179 -6.873l-5 -4.867l6.9 -1l3.086 -6.253l3.086 6.253l6.9 1l-5 4.867l1.179 6.873z",
        ],
        Icon::Bookmarks => &[
            "M18 7v14l-6 -4l-6 4v-14a4 4 0 0 1 4 -4h4a4 4 0 0 1 4 4z",
        ],
        Icon::Pip => &[
            "M11 19h-6a2 2 0 0 1 -2 -2v-10a2 2 0 0 1 2 -2h14a2 2 0 0 1 2 2v4",
            "M14 14m0 1a1 1 0 0 1 1 -1h5a1 1 0 0 1 1 1v3a1 1 0 0 1 -1 1h-5a1 1 0 0 1 -1 -1z",
        ],
        Icon::History => &[
            "M12 8l0 4l2 2",
            "M3.05 11a9 9 0 1 1 .5 4m-.5 5v-5h5",
        ],
        Icon::Code => &[
            "M7 8l-4 4l4 4",
            "M17 8l4 4l-4 4",
            "M14 4l-4 16",
        ],
        Icon::WinMin => &[
            "M5 12l14 0",
        ],
        Icon::WinMax => &[
            "M3 3m0 2a2 2 0 0 1 2 -2h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2z",
        ],
        Icon::WinRestore => &[
            "M7 7m0 2.667a2.667 2.667 0 0 1 2.667 -2.667h8.666a2.667 2.667 0 0 1 2.667 2.667v8.666a2.667 2.667 0 0 1 -2.667 2.667h-8.666a2.667 2.667 0 0 1 -2.667 -2.667z",
            "M4.012 16.737a2.005 2.005 0 0 1 -1.012 -1.737v-10c0 -1.1 .9 -2 2 -2h10c.75 0 1.158 .385 1.5 1",
        ],
        Icon::WinClose => &[
            "M18 6l-12 12",
            "M6 6l12 12",
        ],
    }
}
