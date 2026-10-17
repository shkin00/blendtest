# PixelLab textures

All of these come from PixelLab's MCP `create_tiles_pro`, using square top-down tiles at 128 px. 128 px is the
largest seamless tile PixelLab makes. It has no seamless 256 px option, and the "Create Texture" tool exists
only in its Aseprite extension, at 16x16. PixelLab's terms govern the output.

Every variation is kept here, named `<job>-tile_<n>.png`. The picks are copied to `assets/<layer>.raw.png`, and
The app makes the alpha noise at runtime.

## Seam scores

`cargo run -- inspect <png>` prints a seam score for each direction: the mean colour step across the wrap edge
divided by the mean step inside the tile. A score of 1 means the edge joins as smoothly as the tile runs inside.

## Job 2576d98f-7c8d-4542-a6c2-1f22bfb157a9

Settings: seed 7, view `top-down`, `outline_mode: segmentation`.

Prompt:

> Seamless straight top-down ground textures that fill the whole tile edge to edge, no objects, no border.
> 1). base soil: dark brown bare damp earth with small pebbles and clods, muted low saturation 2). dirt: dry
> grey-brown packed dirt with thin cracks and fine gravel, muted low saturation 3). grass: dense short grass,
> muted olive and yellow-green blades, low saturation

| Tile | Terrain | Notes |
|---|---|---|
| 0 | soil | **Picked for soil.** Seam score 1.89 across, 2.69 down. |
| 1 | soil | Darker blotches. |
| 2 | dirt | Almost flat grey. |
| 3 | dirt | Almost flat grey, with a hard vertical seam (seam score 18.9 across). |
| 4 | grass | **Picked for grass.** Seam score 1.29 across, 2.26 down; a faint band shows where it repeats vertically. |
| 5 | grass | Clumpier. |

## Job caec5f02-7565-45ba-b2d4-7979611734a1

Settings: seed 11. Style reference: soil tile 0 above, with `color_palette: false`, so it matched the detail
and shading but not the colours.

Prompt:

> Seamless straight top-down ground texture that fills the whole tile edge to edge, no objects, no border.
> 1). dirt: light dusty grey-brown packed dirt road surface with fine gravel, small stones, thin cracks and faint
> tyre ruts, muted low saturation, clearly lighter than dark soil 2). dirt: light dusty grey-brown packed dirt
> with scattered small stones and cracks, muted low saturation

The colours came out as a saturated sandy tan, not the muted grey-brown asked for. With `color_palette: false`
the prompt alone sets the colours, and PixelLab did not hold to "muted". Next time, pass `create_tiles_pro`'s
`palette` with the wanted hex colours: every pixel is then one of them.

| Tile | Notes |
|---|---|
| 0, 1 | Tyre ruts. |
| 2 | A blue-grey depth band along the bottom edge: the style tile's view came through as low top-down. |
| 3 | **Picked for dirt.** Pebbles and darker blotches; seam score 2.00 across, 2.28 down. |
| 4 | Similar to tile 3. |
| 5 | Orange and cracked. |
