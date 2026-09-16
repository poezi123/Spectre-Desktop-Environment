precision highp float;

varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;

uniform float spectre_radius;
uniform float spectre_spread;
uniform float spectre_strength;
uniform float spectre_drop;

float sd_round_box(vec2 p, vec2 half_size, float radius) {
    float r = min(radius, min(half_size.x, half_size.y));
    vec2 q = abs(p) - half_size + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

void main() {
    vec2 px = v_coords * size;
    vec2 half_size = size * 0.5;
    vec2 p = px - half_size - vec2(0.0, spectre_drop);
    vec2 inner = max(half_size - vec2(spectre_spread), vec2(1.0));
    float d = sd_round_box(p, inner, spectre_radius);
    float shade = 1.0 - smoothstep(0.0, max(spectre_spread, 1.0), d);
    shade = shade * shade;
    gl_FragColor = vec4(0.0, 0.0, 0.0, shade * spectre_strength * alpha);
}
