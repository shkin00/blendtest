# blendtest

A test for Krust (`C:\unwork\mmo`): can grayscale weightmaps blend pixel-art terrain layers without breaking
the pixel look? Krust's `docs/CONCEPT.md` (Terrain blending) rejects weight masks on the grounds that their
smooth gradients break it. This app tries the alpha-clipping variant:

- Each layer's albedo is read nearest, texel by texel. Its alpha is a separate one-byte-per-texel map, made at
  runtime from grayscale noise and also read texel by texel.
- Each layer above the soil has its own mask, one value per tile corner (16 px), read bilinearly at the
  **texel's centre**.
- A texel shows when `alpha > 1 - mask`, which is a hard test. UFO's `alphaClip`
  (`content/client/shaders/common.h:41`) and the same technique in another UE4 project use the same remap
  with a smoothstep of spread 0.3 and filtered textures. The Spread slider covers both.

Layers are drawn bottom to top: soil (opaque), then dirt, then grass, each over what is below.

## Pass criteria

At spread 0 and integer zoom:

1. Every pixel is one of its own texel's three layer colours.
2. Every texel is drawn whole.

`tests/pixel_look.rs` checks both on GPU renders. It also checks that each way of breaking them fails the same
check: soft clipping produces colours no layer has, and reading the mask per pixel cuts texels.

## Use

```
cargo run --release                 # paint; --demo starts with the demo painting
cargo run -- placeholders [--force] # write assets/<layer>.raw.png placeholder colours
cargo run -- screenshot out.png [--zoom 3 --spread 0 --no-snap --mode winner --noise cellular --noise-size 256]
cargo run -- inspect a.png b.png --sheet sheet.png   # seam scores, and a sheet of each texture 2 x 2
cargo test
```

The textures are PixelLab's, 128 px each; `assets/pixellab/README.md` records the jobs, prompts and picks.
`FINDINGS.md` holds the results so far.

`assets/<layer>.raw.png` holds the colours; their own alpha channel is ignored. The app's Reload textures
button reads them again.

Each layer has its own alpha noise, set in the panel's "Alpha noise" section for the selected layer. A map is
made again on every change; nothing is saved. The options:

- **Kind:**
  - FBM: blobs.
  - White: per-texel dither.
  - Cellular: Voronoi cells, each one value, so a cell appears whole.
  - Luminance: the albedo's brightness, with equal colours ordered by FBM.
- **Size:** the albedo's, so the noise repeats with the art, or 32 to 512 texels so it does not. Luminance is
  always the albedo's size.
- **Feature or cell size, octaves, persistence and seed.**
- **Equalise:** coverage follows the mask linearly; equal values stay equal.
- **Invert:** the lowest values show first.

A 512 px map takes tens of milliseconds in a release build, so use `--release` when dragging the noise sliders.

Painting controls:

- Left drag paints the selected layer. Right drag or Ctrl+left drag erases.
- The wheel zooms and middle drag pans.
- The wheel over a slider moves it one step per notch, and the panel does not scroll then. Linear sliders
  move by a fixed step; radius, strength and feature move by ×1.1.
- Keys 1 to 3 pick a layer; `[` and `]` resize the brush.
- View: Final, Mask (the clip's input), Alpha (the noise), Winner (which layer each texel shows).
