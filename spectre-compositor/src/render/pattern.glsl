// The Spectre Pattern: animated topographic contour lines.
//
// Compiled by GlesRenderer::compile_custom_pixel_shader, so it runs against
// GLSL ES 1.00 with smithay's texture vertex shader. That gives us `v_coords`
// (0..1 across the element) and the `size` / `alpha` uniforms; everything with
// a `spectre_` prefix is ours.
//
// Deliberately free of dFdx/dFdy: derivatives need GL_OES_standard_derivatives,
// which is exactly the extension a weak GPU or a VM's software GL is likely to
// be missing, and the project targets those machines.

precision mediump float;

varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;

// Scroll offset. Held at 0.0 when the pattern is configured static.
uniform float spectre_phase;
// Distance between contour lines, in device pixels.
uniform float spectre_spacing;
// Contour line thickness, in device pixels.
uniform float spectre_line_width;
// Straight-alpha contour colours, sampled as a loop.
uniform vec4 spectre_line_0;
uniform vec4 spectre_line_1;
uniform vec4 spectre_line_2;
uniform vec4 spectre_line_3;
// Where the colour loop stands, 0..1, and how many loops span the surface.
uniform float spectre_color_phase;
uniform float spectre_color_span;
// Straight-alpha colour of the surface underneath.
uniform vec4 spectre_bg;

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

// A polynomial hash rather than the usual sin() one: a transcendental per
// noise sample, sixteen of them per pixel, is what made this shader cost a
// tenth of a second per frame on software GL.
float hash(vec2 p) {
    vec3 q = fract(vec3(p.x, p.y, p.x) * 0.1031);
    q += dot(q, vec3(q.y, q.z, q.x) + 33.33);
    return fract((q.x + q.y) * q.z);
}

// Value noise with a smoothstep interpolant.
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

// Four octaves is the point where the ridges stop looking like blobs and start
// looking like a contour map. A fifth is not visible at this line intensity.
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

void main() {
    vec2 px = v_coords * size;

    // One noise cell every ~6 contour spacings keeps the ridges broad and the
    // lines readable at panel height as well as at full-screen size.
    vec2 q = px / max(spectre_spacing * 6.0, 1.0);
    float height = fbm(q + vec2(spectre_phase, 0.0));

    // Slice the height field into levels; the isolines are the level crossings.
    float levels = height * 16.0;
    float dist = abs(fract(levels) - 0.5);

    // Half line width expressed in level units. Because the level gradient is
    // steeper on slopes, lines naturally tighten there, which is what makes the
    // result read as topography instead of as a plain ripple.
    float half_width = clamp(spectre_line_width / max(spectre_spacing, 1.0), 0.004, 0.4);
    float feather = half_width * 0.9 + 0.015;
    float line = 1.0 - smoothstep(half_width, half_width + feather, dist);

    vec4 line_color = spectre_line_at(v_coords.x * spectre_color_span + spectre_color_phase);
    float coverage = line * line_color.a;
    vec3 rgb = mix(spectre_ground(spectre_bg.rgb, line_color.rgb), line_color.rgb, coverage);
    float a = spectre_bg.a + (1.0 - spectre_bg.a) * coverage;

    // smithay's GLES frame blends premultiplied colours.
    gl_FragColor = vec4(rgb * a, a) * alpha;
}
