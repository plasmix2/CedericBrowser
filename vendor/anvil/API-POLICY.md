# ANVIL public API, semver and changelog policy

Applies to `anvil_core`, `anvil_ui` and `anvl_gpu`. All three share the workspace version in `Cargo.toml`.

## What is public API

- Items reachable from the crate root: `pub use` re-exports in `lib.rs` and the `pub mod` paths listed there.
- Anything not reachable that way, or marked `#[doc(hidden)]`, is internal and may change in any release.
- Items used only by the apps in `apps/` must still be public if an app outside the workspace could need them; otherwise make them `pub(crate)`.

## Semver rule

- **Before 1.0 (now, 0.x):** a breaking change to public API bumps the minor version (0.3 to 0.4). Additions and fixes bump the patch.
- **From 1.0:** breaking change = major, additive = minor, fix = patch.
- Breaking means: removing or renaming a public item, changing a signature, a field, an enum variant set, or a trait's required items, or changing documented behavior.
- Adding a variant to a public enum is breaking unless the enum is `#[non_exhaustive]`. New public enums and structs with public fields should be `#[non_exhaustive]`.
- Raising the minimum supported Rust version is a minor bump.

## Deprecation

Mark with `#[deprecated(since = "x.y.z", note = "use ... instead")]` for at least one minor version before removal, as in TODO.md section 18.

## Changelog

- One `CHANGELOG.md` at the repo root, newest first, one section per release.
- Each entry is under **Added**, **Changed**, **Deprecated**, **Removed**, **Fixed** or **Breaking**.
- A change that touches public API is not merged without a changelog line.
- Breaking entries say what to do instead.

## Status of the freeze

Not done. The crates expose about 960 `pub` items and 51 `pub(crate)`, and apps import from most `anvil_ui` modules (`runner`, `platform`, `element`, `ui`, `theme`, `layout`, `components`, ...). The freeze list should be decided after TODO.md section 0, item "Remove app-specific code from the engine", since that item changes the surface.
