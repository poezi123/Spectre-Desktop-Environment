// The window frame: a rounded title bar and border, with the client area cut
// out of it.
//
// One shader rather than a pile of rectangles, because rectangles cannot have
// rounded corners. It draws, from the outside in:
//
//   * a hairline along the rounded outer edge,
//   * the title bar band, filled and carrying the Spectre Pattern,
//   * nothing at all where the client surface goes.
//
// Compiled by GlesRenderer::compile_custom_pixel_shader, so `size` and `alpha`
// come from the renderer and `v_coords` runs 0..1 across the element.

precision mediump float;

varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;

// Corner radius, in device pixels.
uniform float spectre_radius;
// Border thickness, in device pixels. Zero draws no hairline.
uniform float spectre_border;
// Height of the title bar band measured from the top, border included.
uniform float spectre_titlebar;
// Title bar fill and border hairline, straight alpha.
uniform vec4 spectre_bg;
uniform vec4 spectre_edge;
// Straight-alpha contour colours, sampled as a loop.
uniform vec4 spectre_line_0;
uniform vec4 spectre_line_1;
uniform vec4 spectre_line_2;
uniform vec4 spectre_line_3;
// Where the colour loop stands, 0..1, and how many loops span the surface.
uniform float spectre_color_phase;
uniform float spectre_color_span;
uniform float spectre_phase;
uniform float spectre_spacing;
uniform float spectre_line_width;

// Signed distance to a rounded box centred on the origin.
float sd_round_box(vec2 p, vec2 half_size, float radius) {
    float r = min(radius, min(half_size.x, half_size.y));
    vec2 q = abs(p) - half_size + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

// A polynomial hash rather than the usual sin() one: a transcendental per
// noise sample, sixteen of them per pixel, is what made this shader cost a
// tenth of a second per frame on software GL.
float hash(vec2 p) {
    vec3 q = fract(vec3(p.x, p.y, p.x) * 0.1031);
    q += dot(q, vec3(q.y, q.z, q.x) + 33.33);
    return fract((q.x + q.y) * q.z);
}

float value_noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    float a = hash(i);
    float b = hash(i + vec2(1.0, 0.0));
    float c = hash(i + vec2(0.0, 1.0));
    float d = hash(i + vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

float fbm(vec2 p) {
    float v = 0.0;
    float amp = 0.5;
    for (int i = 0; i < 4; i++) {
        v += amp * value_noise(p);
        p *= 2.03;
        amp *= 0.5;
    }
    return v;
}

// The contour colour at `t` along the loop, wrapping. Twin of Pattern::line_at.
vec4 spectre_line_at(float t) {
    float u = fract(t) * 4.0;
    float i = floor(u);
    float f = u - i;
    vec4 a = i < 0.5 ? spectre_line_0 : (i < 1.5 ? spectre_line_1 : (i < 2.5 ? spectre_line_2 : spectre_line_3));
    vec4 b = i < 0.5 ? spectre_line_1 : (i < 1.5 ? spectre_line_2 : (i < 2.5 ? spectre_line_3 : spectre_line_0));
    return mix(a, b, f);
}

// The ground the lines sit on, tinted by the colour passing overhead: near
// black at the cyan end of the accent, a deep magenta at the other. Twin of
// Pattern::ground.
vec3 spectre_ground(vec3 base, vec3 line) {
    return mix(base, line * 0.20, 0.30);
}

// Contour line coverage at a point, matching pattern.glsl.
float contour(vec2 px) {
    if (spectre_spacing < 0.5) {
        return 0.0;
    }
    vec2 q = px / max(spectre_spacing * 6.0, 1.0);
    float height = fbm(q + vec2(spectre_phase, 0.0));
    float levels = height * 16.0;
    float dist = abs(fract(levels) - 0.5);
    float half_width = clamp(spectre_line_width / max(spectre_spacing, 1.0), 0.004, 0.4);
    float feather = half_width * 0.9 + 0.015;
    return 1.0 - smoothstep(half_width, half_width + feather, dist);
}

void main() {
    vec2 px = v_coords * size;
    vec2 half_size = size * 0.5;
    vec2 p = px - half_size;

    // One pixel of feathering: enough to take the stair-steps off a curve
    // without making the edge look soft.
    const float AA = 0.8;

    float outer = 1.0 - smoothstep(-AA, AA, sd_round_box(p, half_size, spectre_radius));
    float inner_radius = max(spectre_radius - spectre_border, 0.0);
    float inner = 1.0 - smoothstep(
        -AA,
        AA,
        sd_round_box(p, half_size - vec2(spectre_border), inner_radius)
    );

    // Below the title bar is the client's business; we draw nothing there.
    float below = step(spectre_titlebar, px.y);
    float bar = inner * (1.0 - below);
    float ring = outer * (1.0 - inner);

    // The contour field is the expensive half of this shader and only the
    // title bar shows it. Computing it for the client area as well - four
    // fifths of a window - and then multiplying it by zero was costing more
    // than everything else in the frame put together.
    vec3 bar_rgb = spectre_bg.rgb;
    if (bar > 0.0) {
        vec4 line_color = spectre_line_at(v_coords.x * spectre_color_span + spectre_color_phase);
        float coverage = contour(px) * line_color.a;
        bar_rgb = mix(spectre_ground(spectre_bg.rgb, line_color.rgb), line_color.rgb, coverage);
    }

    vec3 rgb = bar_rgb * bar + spectre_edge.rgb * ring;
    float a = spectre_bg.a * bar + spectre_edge.a * ring;

    gl_FragColor = vec4(rgb * a, a) * alpha;
}
