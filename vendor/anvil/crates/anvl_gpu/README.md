# ANVL_GPU

Everything Anvil draws, and the assets it draws with. `anvil_ui` builds a `DrawList` each frame;
`Renderer` turns it into GPU work.

Direct dependencies: `wgpu` (the GPU), `cosmic-text` (text shaping, with swash rasterizing),
`bytemuck` and `glam`. Everything else is Anvil's own code.

| Part | Module | What it does |
|---|---|---|
| Renderer | `render` | Rounded shapes with borders, soft shadows and gradients; strokes; icons; images; text; backdrop blur. One pass per frame, three when glass is on screen. Draws to a window or headless (`render_to_frame`) |
| Draw list | `draw` | Per-layer commands, a clip stack, frame opacity, gradients (`RectStyle::linear`, `radial`), strokes (`stroke`, `arc`) |
| Paths | `path` | Lines and quadratic/cubic Bézier curves, flattened within 0.25 px |
| Text | `text`, `glyphs` | Shaping by cosmic-text; Anvil's own glyph atlas (shelf packed, grows to 4096 px) and clipping |
| Fonts | `assets/fonts` | Inter (400/500/600, italics), Noto Sans (400/500), Fira Mono 500, compiled in |
| Icons | `icons`, `assets/icons.png` | Lucide icons baked into one atlas. Re-bake: `cargo run -p anvl_gpu --example bake_icons` |
| Raster | `raster` | CPU polygon fill (nonzero and even-odd, anti-aliased) for logos made at exact pixel sizes |
| Images | `image` | Anvil's own PNG (every type, interlaced, 16-bit), JPEG (baseline and extended), GIF (first frame) and BMP decoders, with its own inflate |
| Video | `video` | The OS decoder: Media Foundation on Windows (DXVA hardware decoding through a D3D11 device), MediaCodec on Android |

GPU backends, by platform: DX12 on Windows (WARP covers machines without a GPU), Vulkan and GLES
on Android and Linux, Metal on macOS.

## Tests

- `tests/headless.rs` draws real frames on the GPU and checks pixels: text and its clip, exact
  colors, blur, gradients, strokes and arcs, shape clipping.
- `tests/images.rs` compares every decoder with Pillow's output (`tests/fixtures`).
- `tests/icons_baked.rs` fails when `assets/icons.png` falls behind the icon sources.
- `cargo test -p anvl_gpu -- --ignored` with `ANVIL_TEST_VIDEO=<file>` plays a real video.

## Not supported yet

- Progressive JPEG (refused with a clear error), WebP, animated GIF frames after the first.
- EXIF rotation of JPEG photos.
- Video on Linux and macOS.
- Filled vector paths on the GPU (strokes and arcs are GPU; fills go through `raster`).
