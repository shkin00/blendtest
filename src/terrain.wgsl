// The terrain: soil, with dirt and then grass over it, each clipped by its own mask against
// its texels' alpha. One triangle covers the viewport; every pixel finds its world texel.

struct Params {
    // The viewport in target pixels: x, y, width, height.
    viewport: vec4<f32>,
    // The world pixel at the viewport's top-left, a multiple of 1 / zoom.
    origin: vec2<f32>,
    // Target pixels per world pixel, a whole number.
    zoom: f32,
    // 0 is the hard clip; UFO uses 0.3.
    spread: f32,
    // The world's size in pixels; the masks cover 0..world.
    world: f32,
    mask_spacing: f32,
    // Mask texels per side.
    mask_side: f32,
    // 0 final, 1 the active layer's mask, 2 its alpha, 3 the layer that won.
    view_mode: u32,
    active_layer: u32,
    // Bit i: layer i is drawn.
    visible: u32,
    // 1: the masks are read at the texel's centre, so a texel is all in or all out. 0: at the
    // pixel's, which cuts texels at zooms above 1.
    snap: u32,
    pad: u32,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var soil: texture_2d<f32>;
@group(0) @binding(2) var dirt: texture_2d<f32>;
@group(0) @binding(3) var grass: texture_2d<f32>;
// Each layer's alpha map, one byte per texel, repeating on its own size.
@group(0) @binding(4) var soil_alpha: texture_2d<f32>;
@group(0) @binding(5) var dirt_alpha: texture_2d<f32>;
@group(0) @binding(6) var grass_alpha: texture_2d<f32>;
@group(0) @binding(7) var dirt_mask: texture_2d<f32>;
@group(0) @binding(8) var grass_mask: texture_2d<f32>;
@group(0) @binding(9) var linear_clamp: sampler;

const BACKGROUND = vec3<f32>(0.08, 0.08, 0.09);
const WINNER_COLOURS = array<vec3<f32>, 3>(
    vec3<f32>(0.55, 0.25, 0.2),
    vec3<f32>(0.85, 0.75, 0.3),
    vec3<f32>(0.25, 0.7, 0.3),
);

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index & 2u) * 2 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

// A texel of a texture that repeats over the world; `at` is never negative.
fn texel(t: texture_2d<f32>, at: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(t));
    return textureLoad(t, at % size, 0);
}

// As blend::texel_alpha: the centre of the byte's 1/256 step.
fn texel_alpha(a: f32) -> f32 {
    return (round(a * 255.0) + 0.5) / 256.0;
}

// As blend::clip. smoothstep with equal edges is undefined, so spread 0 is its own branch.
fn clip(mask: f32, alpha: f32, spread: f32) -> f32 {
    if spread <= 0.0 {
        return select(0.0, 1.0, alpha > 1.0 - mask);
    }
    let centre = 1.0 + spread - mask * (1.0 + 2.0 * spread);
    return smoothstep(centre - spread, centre + spread, alpha);
}

// As Mask::uv: corner i of the mask grid is the centre of mask texel i.
fn mask_at(t: texture_2d<f32>, world: vec2<f32>) -> f32 {
    let uv = (world / params.mask_spacing + 0.5) / params.mask_side;
    return textureSampleLevel(t, linear_clamp, uv, 0.0).r;
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let world = params.origin + (position.xy - params.viewport.xy) / params.zoom;
    if any(world < vec2<f32>(0.0)) || any(world >= vec2<f32>(params.world)) {
        return vec4<f32>(BACKGROUND, 1.0);
    }
    let at = vec2<i32>(floor(world));
    let read_at = select(world, vec2<f32>(at) + 0.5, params.snap != 0u);
    var colours = array<vec4<f32>, 3>(texel(soil, at), texel(dirt, at), texel(grass, at));
    var alphas = array<f32, 3>(
        texel(soil_alpha, at).r,
        texel(dirt_alpha, at).r,
        texel(grass_alpha, at).r,
    );
    var masks = array<f32, 3>(1.0, mask_at(dirt_mask, read_at), mask_at(grass_mask, read_at));

    var colour = BACKGROUND;
    if (params.visible & 1u) != 0u {
        colour = colours[0].rgb;
    }
    var winner = 0u;
    for (var layer = 1u; layer < 3u; layer++) {
        if (params.visible & (1u << layer)) == 0u {
            continue;
        }
        let cover = clip(masks[layer], texel_alpha(alphas[layer]), params.spread);
        if cover >= 1.0 {
            // Exactly the texel's colour, never a mix that rounds a byte the other way.
            colour = colours[layer].rgb;
        } else if cover > 0.0 {
            colour = mix(colour, colours[layer].rgb, cover);
        }
        if cover >= 0.5 {
            winner = layer;
        }
    }

    switch params.view_mode {
        case 1u: {
            return vec4<f32>(vec3<f32>(masks[params.active_layer]), 1.0);
        }
        case 2u: {
            return vec4<f32>(vec3<f32>(alphas[params.active_layer]), 1.0);
        }
        case 3u: {
            var winners = WINNER_COLOURS;
            return vec4<f32>(winners[winner], 1.0);
        }
        default: {
            return vec4<f32>(colour, 1.0);
        }
    }
}
