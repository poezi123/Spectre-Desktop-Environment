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
// Contour line colours at the left and right edges.
uniform vec4 spectre_line_a;
uniform vec4 spectre_line_b;
uniform float spectre_phase;
uniform float spectre_spacing;
uniform float spectre_line_width;

// Signed distance to a rounded box centred on the origin.
float sd_round_box(vec2 p, vec2 half_size, float radius) {
    float r = min(radius, min(half_size.x, half_size.y));
    vec2 q = abs(p) - half_size + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453123);
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

// Contour line coverage at a point, matching pattern.glsl.
float contour(vec2 px) {
    if (spectre_spacing < 0.5) {
        return 0.0;
    }
    vec2 q = px / max(spectre_spacing * 6.0, 1.0);
    float height = fbm(q + vec2(spectre_phase, spectre_phase * 0.6));
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

    vec4 line_color = mix(spectre_line_a, spectre_line_b, clamp(v_coords.x, 0.0, 1.0));
    float coverage = contour(px) * line_color.a;
    vec3 bar_rgb = mix(spectre_bg.rgb, line_color.rgb, coverage);

    vec3 rgb = bar_rgb * bar + spectre_edge.rgb * ring;
    float a = spectre_bg.a * bar + spectre_edge.a * ring;

    gl_FragColor = vec4(rgb * a, a) * alpha;
}
