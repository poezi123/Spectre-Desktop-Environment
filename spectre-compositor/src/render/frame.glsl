precision highp float;

varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;

uniform float spectre_radius;
uniform float spectre_border;
uniform float spectre_titlebar;
uniform vec4 spectre_bg;
uniform vec4 spectre_edge;
uniform vec4 spectre_line_0;
uniform vec4 spectre_line_1;
uniform vec4 spectre_line_2;
uniform vec4 spectre_line_3;
uniform float spectre_color_phase;
uniform float spectre_color_span;
uniform float spectre_phase;
uniform float spectre_spacing;
uniform float spectre_line_width;

float sd_round_box(vec2 p, vec2 half_size, float radius) {
    float r = min(radius, min(half_size.x, half_size.y));
    vec2 q = abs(p) - half_size + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

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

vec4 spectre_line_at(float t) {
    float u = fract(t) * 4.0;
    float i = floor(u);
    float f = u - i;
    vec4 a = i < 0.5 ? spectre_line_0 : (i < 1.5 ? spectre_line_1 : (i < 2.5 ? spectre_line_2 : spectre_line_3));
    vec4 b = i < 0.5 ? spectre_line_1 : (i < 1.5 ? spectre_line_2 : (i < 2.5 ? spectre_line_3 : spectre_line_0));
    return mix(a, b, f);
}

vec3 spectre_ground(vec3 base, vec3 line) {
    return mix(base, line * 0.20, 0.30);
}

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

    const float AA = 0.8;

    float outer = 1.0 - smoothstep(-AA, AA, sd_round_box(p, half_size, spectre_radius));
    float inner_radius = max(spectre_radius - spectre_border, 0.0);
    float inner = 1.0 - smoothstep(
        -AA,
        AA,
        sd_round_box(p, half_size - vec2(spectre_border), inner_radius)
    );

    float below = step(spectre_titlebar, px.y);
    float bar = inner * (1.0 - below);
    float ring = outer * (1.0 - inner);

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
