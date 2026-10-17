# Findings

These findings are for Krust's terrain-blending decision (`C:\unwork\mmo\docs\CONCEPT.md`, Terrain blending).
That decision rejects weight masks because "smooth gradients break the pixel look". The final verdict on the
look still waits on a hand test in the app.

Status as of 2026-10-17: the measured results below are in. The look itself has not been judged yet.

## Measured

- **The pixel grid holds.** The setup is a hard clip (`alpha > 1 - mask`), nearest albedo, and a bilinear mask
  read at each texel's centre. With it, every pixel is one of its own texel's three layer colours and every
  texel is drawn whole.
  - Checked on GPU renders at zoom 1, 3, 4 and 6, with both the placeholder textures and the PixelLab ones.
  - The whole 1024x1024 world at 1x had 0 off-texel pixels and 0 split texels.
  - The GPU picks the same layer as the CPU model.
  - The tests are in `tests/pixel_look.rs`.
- **The established soft clip breaks the grid.** UFO (`common.h:41`) and another UE4 project use a smoothstep
  with spread 0.3 over filtered textures. On the PixelLab textures at 3x, 103,416 of 614,400 pixels (17%)
  were in colours none of their layers has.
- **The mask has to be read at the texel centre.** On the PixelLab textures at 6x, reading the mask once per
  screen pixel let the clip edge cut 94 of 10,240 texels in one crop.
- **In Krust that read probably comes free.** Krust draws into a 640x360 target at one world pixel per target
  pixel, with the camera rounded to whole pixels (`krust-client/src/tiles.wgsl:10`). A per-pixel mask read
  therefore lands on texel centres, provided the layer textures are mapped at one texel per world pixel on
  whole coordinates. That is how `src/terrain.wgsl` here reads them. Krust's current tile pass samples atlas
  UVs instead, so this condition has not been checked in Krust.
- **The masks fit Krust's corner map.** One mask value per 16 px tile corner is the density of Krust's corner
  map, and it is the app's default.

## Seen, not yet judged

1. **The noise repeats with its texture.** With a 32 px texture, the clip edge shows a regular 32 px grid
   across the world. 128 px textures hide it. The alpha is now its own map, so its size can differ from the
   albedo's, which breaks the shared repeat.
2. **The noise ignores the art.** Equalised FBM alpha cuts pebbles and blades in half along the edge. The edge
   reads as blobs of the upper layer, not as drawn shapes. Two possible remedies:
   - Author the alpha per texture as a height, so a pebble appears or disappears whole, as UE4 terrain
     materials often do with a height texture per layer.
   - Derive the alpha from the albedo's own shapes. The Luminance kind does this by brightness. On the
     PixelLab grass, the edge then grows blade by blade, because the bright blades show first.
3. **PixelLab limits.** The largest seamless tile is 128 px. The seams are good but not perfect (scores in
   `assets/pixellab/README.md`). A style-matched dirt still came out more saturated than asked.

## Still open

- **The hand test.** Paint with the brush at zoom 2 to 4, compare spread 0 against 0.3, and try mask densities
  4 to 32 px.
- **The grass repeat band.** Fill the grass layer and look for a horizontal band every 128 px. A 2x2 sheet of
  the grass texture shows a faint one, and the demo painting is too sparse to show it.
- **The alpha kinds.** Compare them by hand: FBM, White, Cellular and Luminance, at the albedo's size and
  others. A hand-authored height map is still untried.
- **Krust's cost.** A noise map is made from its options (and, for Luminance, the albedo) at bake or load
  time, so it ships nothing. Only a hand-authored height would add an asset per terrain.
