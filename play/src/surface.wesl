import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
};

struct SurfaceParams {
    // x = fade (0..1), y = mode (0 opaque, +1 fade in, -1 fade out), z = grain_scale, w = unused
    fade_mode: vec4<f32>,
    // xyz = world-locked offset in renderer units (eye * scale mod period), w = period
    eye_mod: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100)
var<uniform> surface: SurfaceParams;

fn bayer4(xy: vec2<f32>) -> f32 {
    let p = vec2<u32>(floor(xy)) & vec2<u32>(3u);
    let idx = p.y * 4u + p.x;
    var m = array<f32, 16>(
         0.5,  8.5,  2.5, 10.5,
        12.5,  4.5, 14.5,  6.5,
         3.5, 11.5,  1.5,  9.5,
        15.5,  7.5, 13.5,  5.5,
    );
    return m[idx] / 16.0;
}

fn hash3(p: vec3<f32>) -> f32 {
    let q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    let r = q + dot(q, q.yzx + 33.33);
    return fract((r.x + r.y) * r.z);
}

fn wrap3(p: vec3<f32>, tile: f32) -> vec3<f32> {
    return p - floor(p / tile) * tile;
}

fn noise3(p: vec3<f32>, tile: f32) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let i0 = wrap3(i, tile);
    let i1 = wrap3(i + vec3<f32>(1.0), tile);
    let n000 = hash3(vec3<f32>(i0.x, i0.y, i0.z));
    let n100 = hash3(vec3<f32>(i1.x, i0.y, i0.z));
    let n010 = hash3(vec3<f32>(i0.x, i1.y, i0.z));
    let n110 = hash3(vec3<f32>(i1.x, i1.y, i0.z));
    let n001 = hash3(vec3<f32>(i0.x, i0.y, i1.z));
    let n101 = hash3(vec3<f32>(i1.x, i0.y, i1.z));
    let n011 = hash3(vec3<f32>(i0.x, i1.y, i1.z));
    let n111 = hash3(vec3<f32>(i1.x, i1.y, i1.z));
    let nx00 = mix(n000, n100, u.x);
    let nx10 = mix(n010, n110, u.x);
    let nx01 = mix(n001, n101, u.x);
    let nx11 = mix(n011, n111, u.x);
    let nxy0 = mix(nx00, nx10, u.y);
    let nxy1 = mix(nx01, nx11, u.y);
    return mix(nxy0, nxy1, u.z) * 2.0 - 1.0;
}

@fragment
fn fragment(
    vertex_output: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    let mode = surface.fade_mode.y;
    if mode > 0.5 {
        if bayer4(vertex_output.position.xy) >= surface.fade_mode.x {
            discard;
        }
    } else if mode < -0.5 {
        if bayer4(vertex_output.position.xy) < surface.fade_mode.x {
            discard;
        }
    }

    var in = vertex_output;
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    let period = max(surface.eye_mod.w, 1.0);
    let wp = in.world_position.xyz + surface.eye_mod.xyz;
    let p_mod = wrap3(wp, period);
    let g1 = noise3(p_mod * 6.0, period * 6.0);
    let g2 = noise3(p_mod * 19.0, period * 19.0);
    let grain = g1 * 0.65 + g2 * 0.35;

    let rough = clamp(in.uv.x, 0.06, 0.98);
    let amp = in.uv.y;

    let shade = clamp(1.0 + grain * amp, 0.80, 1.20);
    pbr_input.material.base_color = vec4<f32>(pbr_input.material.base_color.rgb * shade, pbr_input.material.base_color.a);
    pbr_input.material.perceptual_roughness = clamp(rough + grain * amp * 0.6, 0.05, 0.98);
    if rough < 0.2 {
        pbr_input.material.reflectance = vec3<f32>(0.65);
        let r1 = noise3(p_mod * 4.0 + vec3<f32>(0.0, 1.7, 0.0), period * 4.0);
        let r2 = noise3(p_mod * 4.0 + vec3<f32>(3.1, 0.0, 2.3), period * 4.0);
        pbr_input.N = normalize(pbr_input.N + vec3<f32>(r1, 0.0, r2) * 0.035);
    }

    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
