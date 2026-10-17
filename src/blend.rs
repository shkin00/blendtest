//! The alpha-clip test. `terrain.wgsl` mirrors these functions; the checks and tests use these.

/// Layers bottom to top. Soil is the opaque base; every other layer has a mask.
pub const LAYERS: [&str; 3] = ["soil", "dirt", "grass"];

/// An 8-bit texel alpha as the clip compares it: the centre of its 1/256 step. It is never 0 or
/// 1, so a mask of 0 hides every texel and a mask of 1 shows every texel.
pub fn texel_alpha(a8: u8) -> f32 {
    (f32::from(a8) + 0.5) / 256.0
}

/// How much of a layer covers a texel with this alpha where its mask is `mask`. This is UFO's
/// `alphaClip` (content/client/shaders/common.h:41, spread fixed at 0.3 there) with the spread as
/// a parameter. Spread 0 is the hard test `alpha > 1 - mask`, which keeps every texel whole.
pub fn clip(mask: f32, alpha: f32, spread: f32) -> f32 {
    // smoothstep with equal edges is undefined in WGSL, so spread 0 is its own branch.
    if spread <= 0.0 {
        return if alpha > 1.0 - mask { 1.0 } else { 0.0 };
    }
    let centre = 1.0 + spread - mask * (1.0 + 2.0 * spread);
    smoothstep(centre - spread, centre + spread, alpha)
}

pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_zero_hides_every_texel_and_mask_one_shows_every_texel() {
        for spread in [0.0, 0.1, 0.3] {
            for a8 in 0..=255 {
                assert_eq!(
                    clip(0.0, texel_alpha(a8), spread),
                    0.0,
                    "a8 {a8} spread {spread}"
                );
                assert_eq!(
                    clip(1.0, texel_alpha(a8), spread),
                    1.0,
                    "a8 {a8} spread {spread}"
                );
            }
        }
    }

    #[test]
    fn a_hard_clip_at_half_shows_the_upper_half_of_the_alphas() {
        for a8 in 0..=255u8 {
            let expected = if a8 >= 128 { 1.0 } else { 0.0 };
            assert_eq!(clip(0.5, texel_alpha(a8), 0.0), expected, "a8 {a8}");
        }
    }

    #[test]
    fn a_hard_clip_shows_mask_times_256_alphas_at_each_step() {
        // With every alpha once, a mask of k/256 shows exactly k of them.
        for k in 0..=256 {
            let mask = k as f32 / 256.0;
            let shown = (0..=255u8)
                .filter(|&a8| clip(mask, texel_alpha(a8), 0.0) == 1.0)
                .count();
            assert_eq!(shown, k, "mask {mask}");
        }
    }

    #[test]
    fn a_hard_clip_gives_only_whole_texels_and_a_soft_one_does_not() {
        let partial = |spread| {
            (0..=255u8).any(|a8| {
                let c = clip(0.5, texel_alpha(a8), spread);
                c > 0.0 && c < 1.0
            })
        };
        assert!(!partial(0.0));
        assert!(partial(0.3));
    }

    #[test]
    fn the_soft_clip_is_ufos_with_its_spread() {
        // UFO: centre = 1.3 - 1.6 * mask, smoothstep over centre -+ 0.3.
        let mask = 0.4;
        let centre = 1.3 - 1.6 * mask;
        for a8 in [0u8, 60, 100, 128, 160, 255] {
            let a = texel_alpha(a8);
            let t = ((a - (centre - 0.3)) / 0.6).clamp(0.0, 1.0);
            let ufo = t * t * (3.0 - 2.0 * t);
            assert!((clip(mask, a, 0.3) - ufo).abs() < 1e-6, "a8 {a8}");
        }
    }
}
