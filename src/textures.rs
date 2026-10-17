//! The layer textures: `assets/<layer>.raw.png` holds the colours, PixelLab's or a placeholder.
//! Their own alpha is not used; each layer's alpha map is made at runtime by `noise::alpha_map`.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use image::{Rgba, RgbaImage};

use crate::blend::LAYERS;
use crate::noise::{self, Fbm, NoiseParams};

/// Placeholder colours, dark to light: a few shades per layer, as pixel art has.
const PALETTES: [[[u8; 3]; 4]; 3] = [
    [[46, 34, 27], [58, 42, 33], [72, 52, 40], [88, 64, 48]],
    [
        [118, 100, 78],
        [138, 118, 92],
        [158, 137, 107],
        [176, 156, 124],
    ],
    [[53, 90, 36], [63, 107, 42], [79, 128, 51], [99, 150, 60]],
];

/// Placeholder sizes differ, so each layer repeats on its own period.
const PLACEHOLDER_SIZES: [u32; 3] = [64, 48, 32];

pub fn assets_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("assets")
}

pub fn raw_path(dir: &Path, layer: usize) -> PathBuf {
    dir.join(format!("{}.raw.png", LAYERS[layer]))
}

/// The colour textures the app draws.
pub fn load(dir: &Path) -> anyhow::Result<[RgbaImage; 3]> {
    let load = |layer| {
        let path = raw_path(dir, layer);
        image::open(&path)
            .with_context(|| format!("load {}", path.display()))
            .map(|image| image.to_rgba8())
    };
    Ok([load(0)?, load(1)?, load(2)?])
}

/// A placeholder texture: every texel one of four shades, picked by blobby noise.
pub fn placeholder(layer: usize, size: u32) -> RgbaImage {
    let params = NoiseParams {
        feature: 5.0,
        octaves: 2,
        seed: 77 + layer as u32,
        ..NoiseParams::default()
    };
    let noise = Fbm::new(size, size, params);
    let values: Vec<f64> = (0..size * size)
        .map(|i| f64::from(noise.at((i % size) as f32, (i / size) as f32)))
        .collect();
    let shades = noise::equalise(&values);
    RgbaImage::from_fn(size, size, |x, y| {
        let [r, g, b] = PALETTES[layer][shades[(y * size + x) as usize] as usize / 64];
        Rgba([r, g, b, 255])
    })
}

/// The placeholders, without touching the disk.
pub fn placeholders() -> [RgbaImage; 3] {
    std::array::from_fn(|layer| placeholder(layer, PLACEHOLDER_SIZES[layer]))
}

/// Write the placeholder raw textures. An existing raw texture is kept unless `force`.
pub fn write_placeholders(dir: &Path, force: bool) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    for (layer, &size) in PLACEHOLDER_SIZES.iter().enumerate() {
        let path = raw_path(dir, layer);
        if path.exists() && !force {
            bail!("{} exists; --force replaces it", path.display());
        }
        placeholder(layer, size)
            .save(&path)
            .with_context(|| format!("write {}", path.display()))?;
        println!("{}", path.display());
    }
    Ok(())
}

/// How visible the wrap seams are: the mean colour step across the left-right edge and across
/// the top-bottom edge, each over the mean step between neighbours inside. Near 1 the texture
/// joins itself as smoothly as it runs inside; well above 1 it shows a seam.
pub fn seam_ratios(image: &RgbaImage) -> (f32, f32) {
    let (width, height) = image.dimensions();
    let step = |a: (u32, u32), b: (u32, u32)| -> f32 {
        let (p, q) = (image.get_pixel(a.0, a.1).0, image.get_pixel(b.0, b.1).0);
        (0..3)
            .map(|c| (f32::from(p[c]) - f32::from(q[c])).abs())
            .sum()
    };
    let mean = |steps: Vec<f32>| steps.iter().sum::<f32>() / steps.len() as f32;
    let inside_across = mean(
        (0..height)
            .flat_map(|y| (0..width - 1).map(move |x| ((x, y), (x + 1, y))))
            .map(|(a, b)| step(a, b))
            .collect(),
    );
    let inside_down = mean(
        (0..width)
            .flat_map(|x| (0..height - 1).map(move |y| ((x, y), (x, y + 1))))
            .map(|(a, b)| step(a, b))
            .collect(),
    );
    let edge_across = mean((0..height).map(|y| step((width - 1, y), (0, y))).collect());
    let edge_down = mean((0..width).map(|x| step((x, height - 1), (x, 0))).collect());
    (edge_across / inside_across, edge_down / inside_down)
}

/// The images side by side, each repeated 2 x 2 so its seams cross its middle, scaled up.
pub fn contact_sheet(images: &[RgbaImage], scale: u32) -> RgbaImage {
    let width = images.iter().map(|image| 2 * image.width() * scale).sum();
    let height = images
        .iter()
        .map(|image| 2 * image.height() * scale)
        .max()
        .unwrap_or(1);
    let mut sheet = RgbaImage::new(width, height);
    let mut left = 0;
    for image in images {
        let (w, h) = image.dimensions();
        for y in 0..2 * h * scale {
            for x in 0..2 * w * scale {
                sheet.put_pixel(left + x, y, *image.get_pixel(x / scale % w, y / scale % h));
            }
        }
        left += 2 * w * scale;
    }
    sheet
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tileable_texture_has_no_seam_and_a_gradient_has_one() {
        // This checks the measure, not the assets: the placeholders wrap by construction and
        // score near 1. The PixelLab picks score about 1.3 to 2.7 (assets/pixellab/README.md).
        for layer in 0..3 {
            let (across, down) = seam_ratios(&placeholder(layer, 32));
            assert!(across < 1.5 && down < 1.5, "layer {layer}: {across} {down}");
        }
        let gradient = RgbaImage::from_fn(64, 64, |x, _| Rgba([x as u8 * 4, 0, 0, 255]));
        let (across, down) = seam_ratios(&gradient);
        assert!(across > 10.0, "{across}");
        // Every row is the same, so nothing changes going down, across the edge or inside.
        assert!(down.is_nan() || down <= 1.0, "{down}");
    }

    #[test]
    fn the_sheet_repeats_each_image_two_by_two() {
        let a = placeholder(0, 32);
        let b = placeholder(1, 16);
        let sheet = contact_sheet(&[a.clone(), b.clone()], 2);
        assert_eq!(sheet.dimensions(), (2 * 64 + 2 * 32, 2 * 64));
        for (x, y) in [(0, 0), (5, 70), (127, 127), (64, 1)] {
            assert_eq!(sheet.get_pixel(x, y), a.get_pixel(x / 2 % 32, y / 2 % 32));
        }
        for (x, y) in [(128, 0), (128 + 40, 50)] {
            assert_eq!(
                sheet.get_pixel(x, y),
                b.get_pixel((x - 128) / 2 % 16, y / 2 % 16)
            );
        }
    }
}
