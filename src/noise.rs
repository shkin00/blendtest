//! The alpha maps: one per layer, generated at runtime from tileable noise and usually
//! equalised to a flat histogram.

use glam::IVec2;
use image::RgbaImage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseKind {
    /// Value-noise octaves: blobs of about `feature` texels.
    Fbm,
    /// Every texel on its own: a dithered edge.
    White,
    /// Cells of about `feature` texels, each one value, so a cell comes or goes whole.
    Cellular,
    /// The albedo's brightness, ties broken by FBM, so texels go in order of brightness.
    Luminance,
}

pub const KINDS: [NoiseKind; 4] = [
    NoiseKind::Fbm,
    NoiseKind::White,
    NoiseKind::Cellular,
    NoiseKind::Luminance,
];

impl NoiseKind {
    pub fn name(self) -> &'static str {
        match self {
            NoiseKind::Fbm => "FBM",
            NoiseKind::White => "White",
            NoiseKind::Cellular => "Cellular",
            NoiseKind::Luminance => "Luminance",
        }
    }
}

/// How a layer's alpha map is made.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoiseParams {
    pub kind: NoiseKind,
    /// The map's side in texels; `None` is the albedo's size, so the noise repeats with the
    /// art. Luminance is always the albedo's size.
    pub size: Option<u32>,
    /// FBM: the size of the largest blobs; Cellular: the size of a cell; in texels.
    pub feature: f32,
    /// FBM: each octave halves the blob size.
    pub octaves: u32,
    /// FBM: each octave's weight relative to the one before.
    pub persistence: f32,
    pub seed: u32,
    /// Spread the values evenly over 0..=255, so a hard clip at mask m shows about m of the
    /// texels. Equal values stay equal.
    pub equalise: bool,
    /// Flip the map. A high alpha shows at a low mask, so without this the highest values show
    /// first (with Luminance, the brightest texels); with it, the lowest.
    pub invert: bool,
}

impl Default for NoiseParams {
    fn default() -> Self {
        Self {
            kind: NoiseKind::Fbm,
            size: None,
            feature: 8.0,
            octaves: 3,
            persistence: 0.5,
            seed: 1,
            equalise: true,
            invert: false,
        }
    }
}

/// One alpha byte per texel, rows top to bottom; it repeats over the world.
#[derive(Clone, Debug, PartialEq)]
pub struct AlphaMap {
    pub width: u32,
    pub height: u32,
    pub values: Vec<u8>,
}

impl AlphaMap {
    /// The alpha at a world texel.
    pub fn get(&self, texel: IVec2) -> u8 {
        let x = texel.x.rem_euclid(self.width as i32) as u32;
        let y = texel.y.rem_euclid(self.height as i32) as u32;
        self.values[(y * self.width + x) as usize]
    }
}

/// A layer's starting noise: the defaults, with a seed of its own so no two layers share noise.
pub fn layer_defaults(layer: usize) -> NoiseParams {
    NoiseParams {
        seed: 1 + 101 * layer as u32,
        ..NoiseParams::default()
    }
}

/// Every layer's alpha map.
pub fn alpha_maps(params: &[NoiseParams; 3], layers: &[RgbaImage; 3]) -> [AlphaMap; 3] {
    std::array::from_fn(|layer| alpha_map(&params[layer], &layers[layer]))
}

/// A layer's alpha map.
pub fn alpha_map(params: &NoiseParams, albedo: &RgbaImage) -> AlphaMap {
    let (width, height) = match (params.kind, params.size) {
        (NoiseKind::Luminance, _) | (_, None) => albedo.dimensions(),
        (_, Some(size)) => (size.max(1), size.max(1)),
    };
    let texels = (0..height).flat_map(|y| (0..width).map(move |x| (x, y)));
    let values: Vec<f64> = match params.kind {
        NoiseKind::Fbm => {
            let noise = Fbm::new(width, height, *params);
            texels
                .map(|(x, y)| f64::from(noise.at(x as f32, y as f32)))
                .collect()
        }
        NoiseKind::White => {
            let seed = params.seed.wrapping_mul(0x2c1b_3c6d);
            texels
                .map(|(x, y)| f64::from(unit(hash(x, y, seed))))
                .collect()
        }
        NoiseKind::Cellular => {
            let cells = Cellular::new(width, height, params);
            texels
                .map(|(x, y)| f64::from(cells.at(i64::from(x), i64::from(y))))
                .collect()
        }
        NoiseKind::Luminance => {
            // Brightness in whole steps of 1 / 255,000; the noise, below one step, only
            // orders texels of equal brightness.
            let noise = Fbm::new(width, height, *params);
            texels
                .map(|(x, y)| {
                    let [r, g, b, _] = albedo.get_pixel(x, y).0;
                    let brightness = 299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b);
                    let tie = 0.999 * f64::from(noise.at(x as f32, y as f32));
                    (f64::from(brightness) + tie) / 255_000.0
                })
                .collect()
        }
    };
    let mut alphas = if params.equalise {
        equalise(&values)
    } else {
        values
            .iter()
            .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect()
    };
    if params.invert {
        alphas.iter_mut().for_each(|a| *a = 255 - *a);
    }
    AlphaMap {
        width,
        height,
        values: alphas,
    }
}

/// FBM that repeats every `width` x `height` texels: the lattice of every octave fits that
/// period a whole number of times.
pub struct Fbm {
    width: f32,
    height: f32,
    cells: (u32, u32),
    params: NoiseParams,
}

impl Fbm {
    pub fn new(width: u32, height: u32, params: NoiseParams) -> Self {
        let cells = |size: u32| (size as f32 / params.feature).round().max(1.0) as u32;
        Self {
            width: width as f32,
            height: height as f32,
            cells: (cells(width), cells(height)),
            params,
        }
    }

    /// The noise at a texel position, in 0..1.
    pub fn at(&self, x: f32, y: f32) -> f32 {
        let mut sum = 0.0;
        let mut total = 0.0;
        let mut weight = 1.0;
        for octave in 0..self.params.octaves.max(1) {
            let cells = (self.cells.0 << octave, self.cells.1 << octave);
            let u = x / self.width * cells.0 as f32;
            let v = y / self.height * cells.1 as f32;
            let seed = self.params.seed.wrapping_mul(0x9e37_79b9) ^ octave;
            sum += weight * value_noise(u, v, cells, seed);
            total += weight;
            weight *= self.params.persistence;
        }
        sum / total
    }
}

/// FBM over a `width` x `height` texture, row by row, that wraps at the edges.
pub fn fbm(width: u32, height: u32, params: NoiseParams) -> Vec<f32> {
    let noise = Fbm::new(width, height, params);
    (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .map(|(x, y)| noise.at(x as f32, y as f32))
        .collect()
}

/// Cells around jittered points, one point per lattice cell, that repeat every `width` x
/// `height` texels. A texel takes the value of its nearest point.
pub struct Cellular {
    size: (i64, i64),
    cells: (i64, i64),
    seed: u32,
}

impl Cellular {
    pub fn new(width: u32, height: u32, params: &NoiseParams) -> Self {
        let cells = |size: u32| (size as f32 / params.feature).round().max(1.0) as i64;
        Self {
            size: (i64::from(width), i64::from(height)),
            cells: (cells(width), cells(height)),
            seed: params.seed.wrapping_mul(0x5bd1_e995),
        }
    }

    /// The value of the cell a texel falls in, in 0..1.
    pub fn at(&self, x: i64, y: i64) -> f32 {
        let cell = (
            self.size.0 as f64 / self.cells.0 as f64,
            self.size.1 as f64 / self.cells.1 as f64,
        );
        // The texel's centre inside the first period.
        let p = (
            x.rem_euclid(self.size.0) as f64 + 0.5,
            y.rem_euclid(self.size.1) as f64 + 0.5,
        );
        let (ci, cj) = ((p.0 / cell.0).floor() as i64, (p.1 / cell.1).floor() as i64);
        let mut nearest = (f64::MAX, 0.0);
        for j in cj - 1..=cj + 1 {
            for i in ci - 1..=ci + 1 {
                // Points past the edge are the far side's, at their unwrapped place.
                let wi = i.rem_euclid(self.cells.0) as u32;
                let wj = j.rem_euclid(self.cells.1) as u32;
                let h = hash(wi, wj, self.seed);
                let jitter = (unit(h), unit(hash(h, 1, self.seed)));
                let point = (
                    (i as f64 + 0.1 + 0.8 * f64::from(jitter.0)) * cell.0,
                    (j as f64 + 0.1 + 0.8 * f64::from(jitter.1)) * cell.1,
                );
                let distance = (p.0 - point.0).powi(2) + (p.1 - point.1).powi(2);
                if distance < nearest.0 {
                    nearest = (distance, unit(hash(wi, wj, self.seed ^ 0x68e3_1da4)));
                }
            }
        }
        nearest.1
    }
}

/// Every value's mid-rank spread over 0..=255: each alpha appears about equally often, and
/// equal values get equal alphas.
pub fn equalise(values: &[f64]) -> Vec<u8> {
    let count = values.len();
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let mut alphas = vec![0; count];
    let mut start = 0;
    while start < count {
        let value = values[order[start]];
        let end = start + order[start..].partition_point(|&i| values[i] == value);
        let middle = (start + end - 1) as f64 / 2.0;
        let alpha = (middle * 256.0 / count as f64).floor() as u8;
        for &index in &order[start..end] {
            alphas[index] = alpha;
        }
        start = end;
    }
    alphas
}

/// Value noise on a lattice of `cells` that wraps, with quintic interpolation.
fn value_noise(u: f32, v: f32, cells: (u32, u32), seed: u32) -> f32 {
    let (u0, v0) = (u.floor(), v.floor());
    let (fu, fv) = (quintic(u - u0), quintic(v - v0));
    let i0 = (u0 as i64).rem_euclid(i64::from(cells.0)) as u32;
    let j0 = (v0 as i64).rem_euclid(i64::from(cells.1)) as u32;
    let (i1, j1) = ((i0 + 1) % cells.0, (j0 + 1) % cells.1);
    let at = |i: u32, j: u32| unit(hash(i, j, seed));
    let top = at(i0, j0) + (at(i1, j0) - at(i0, j0)) * fu;
    let bottom = at(i0, j1) + (at(i1, j1) - at(i0, j1)) * fu;
    top + (bottom - top) * fv
}

fn quintic(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn unit(hash: u32) -> f32 {
    hash as f32 / u32::MAX as f32
}

fn hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h = x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ seed;
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^ (h >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    use std::collections::HashSet;

    /// Four colours in 4 x 4 blocks, as pixel art has few colours.
    fn albedo(size: u32) -> RgbaImage {
        let colours = [[40, 40, 40], [90, 80, 70], [150, 140, 120], [210, 200, 190]];
        RgbaImage::from_fn(size, size, |x, y| {
            let [r, g, b] = colours[((x / 4 + y / 4) % 4) as usize];
            Rgba([r, g, b, 255])
        })
    }

    fn with(kind: NoiseKind) -> NoiseParams {
        NoiseParams {
            kind,
            ..NoiseParams::default()
        }
    }

    #[test]
    fn equalised_alphas_are_flat_and_keep_the_order() {
        let values: Vec<f64> = fbm(256, 256, NoiseParams::default())
            .into_iter()
            .map(f64::from)
            .collect();
        let alphas = equalise(&values);
        let mut histogram = [0usize; 256];
        for &a in &alphas {
            histogram[a as usize] += 1;
        }
        assert!(histogram.iter().all(|&n| n == 256), "{histogram:?}");
        for (i, j) in [(0, 1), (10, 5000), (300, 65535), (40000, 7)] {
            if values[i] < values[j] {
                assert!(alphas[i] <= alphas[j], "{i} {j}");
            }
        }
    }

    #[test]
    fn equal_values_get_equal_alphas() {
        let alphas = equalise(&[0.5, 0.2, 0.5, 0.9, 0.5, 0.1]);
        assert_eq!(alphas[0], alphas[2]);
        assert_eq!(alphas[0], alphas[4]);
        assert!(alphas[5] < alphas[1] && alphas[1] < alphas[0] && alphas[0] < alphas[3]);
    }

    #[test]
    fn noise_repeats_with_the_texture_and_has_no_seam() {
        for (width, height) in [(32, 32), (256, 256), (48, 32)] {
            let params = NoiseParams {
                feature: 6.0,
                seed: 9,
                ..NoiseParams::default()
            };
            let noise = Fbm::new(width, height, params);
            let (w, h) = (width as f32, height as f32);
            for step in 0..40 {
                let (x, y) = (step as f32 * 1.37, step as f32 * 0.91);
                let here = noise.at(x, y);
                assert!((noise.at(x + w, y) - here).abs() < 1e-4, "{width}x{height}");
                assert!((noise.at(x, y + h) - here).abs() < 1e-4, "{width}x{height}");
                // Just inside the far edge is next to the near edge.
                assert!((noise.at(w - 0.001, y) - noise.at(0.0, y)).abs() < 1e-2);
                assert!((noise.at(x, h - 0.001) - noise.at(x, 0.0)).abs() < 1e-2);
            }
        }
    }

    #[test]
    fn a_seed_gives_the_same_noise_and_another_seed_other_noise() {
        let image = albedo(64);
        for kind in KINDS {
            let params = with(kind);
            let other = NoiseParams { seed: 2, ..params };
            assert_eq!(
                alpha_map(&params, &image),
                alpha_map(&params, &image),
                "{kind:?}"
            );
            assert_ne!(
                alpha_map(&params, &image),
                alpha_map(&other, &image),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_map_is_the_albedo_size_unless_a_size_is_set() {
        let image = albedo(64);
        for kind in [NoiseKind::Fbm, NoiseKind::White, NoiseKind::Cellular] {
            let map = alpha_map(&with(kind), &image);
            assert_eq!((map.width, map.height, map.values.len()), (64, 64, 64 * 64));
            let sized = NoiseParams {
                size: Some(256),
                ..with(kind)
            };
            let map = alpha_map(&sized, &image);
            assert_eq!(
                (map.width, map.height, map.values.len()),
                (256, 256, 256 * 256)
            );
        }
        let luminance = NoiseParams {
            size: Some(256),
            ..with(NoiseKind::Luminance)
        };
        assert_eq!(alpha_map(&luminance, &image).width, 64);
    }

    #[test]
    fn a_map_repeats_over_the_world() {
        let map = alpha_map(&with(NoiseKind::White), &albedo(32));
        for texel in [IVec2::new(3, 5), IVec2::new(31, 0)] {
            assert_eq!(map.get(texel), map.get(texel + IVec2::new(32, -64)));
        }
    }

    #[test]
    fn white_noise_varies_texel_to_texel() {
        let map = alpha_map(&with(NoiseKind::White), &albedo(64));
        let steps = map
            .values
            .windows(2)
            .filter(|w| w[0].abs_diff(w[1]) > 32)
            .count();
        // Neighbours are unrelated: most steps are large. FBM's are small.
        assert!(steps > map.values.len() / 2, "{steps}");
        let smooth = alpha_map(&with(NoiseKind::Fbm), &albedo(64));
        let fbm_steps = smooth
            .values
            .windows(2)
            .filter(|w| w[0].abs_diff(w[1]) > 32)
            .count();
        // FBM's finest default octave is one texel per cell here, so it has some grain too.
        assert!(fbm_steps < steps / 2, "{fbm_steps} {steps}");
    }

    #[test]
    fn a_cell_comes_or_goes_whole() {
        let params = NoiseParams {
            feature: 8.0,
            ..with(NoiseKind::Cellular)
        };
        let cells = Cellular::new(64, 64, &params);
        for (x, y) in [(0, 0), (13, 40), (63, 63), (-5, 70)] {
            assert_eq!(cells.at(x + 64, y), cells.at(x, y));
            assert_eq!(cells.at(x, y - 64), cells.at(x, y));
        }
        let map = alpha_map(&params, &albedo(64));
        let distinct: HashSet<u8> = map.values.iter().copied().collect();
        // 8 x 8 cells of 8 texels: at most one alpha each, and most cells differ.
        assert!(
            distinct.len() <= 64 && distinct.len() > 32,
            "{}",
            distinct.len()
        );
    }

    #[test]
    fn luminance_shows_bright_texels_first_and_breaks_ties_with_noise() {
        // Left half dark, right half bright; each half one colour.
        let image = RgbaImage::from_fn(64, 64, |x, _| {
            if x < 32 {
                Rgba([40, 40, 40, 255])
            } else {
                Rgba([200, 200, 200, 255])
            }
        });
        let map = alpha_map(&with(NoiseKind::Luminance), &image);
        let dark: Vec<u8> = (0..64 * 64)
            .filter(|i| i % 64 < 32)
            .map(|i| map.values[i])
            .collect();
        let bright: Vec<u8> = (0..64 * 64)
            .filter(|i| i % 64 >= 32)
            .map(|i| map.values[i])
            .collect();
        assert!(dark.iter().max() <= bright.iter().min());
        let spread: HashSet<u8> = dark.iter().copied().collect();
        assert!(spread.len() > 64, "{}", spread.len());
    }

    #[test]
    fn invert_flips_the_alphas() {
        let image = albedo(32);
        for kind in KINDS {
            let map = alpha_map(&with(kind), &image);
            let inverted = alpha_map(
                &NoiseParams {
                    invert: true,
                    ..with(kind)
                },
                &image,
            );
            assert!(
                map.values
                    .iter()
                    .zip(&inverted.values)
                    .all(|(a, b)| *a == 255 - *b),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn without_equalising_the_values_are_scaled_to_bytes() {
        let params = NoiseParams {
            equalise: false,
            ..NoiseParams::default()
        };
        let map = alpha_map(&params, &albedo(32));
        let values = fbm(32, 32, params);
        assert!(
            map.values
                .iter()
                .zip(values)
                .all(|(&a, v)| a == (f64::from(v) * 255.0).round() as u8)
        );
    }
}
