//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision mediump float;

#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

uniform vec2 spectre_size;
uniform vec2 spectre_window_min;
uniform vec2 spectre_window_max;
uniform vec4 spectre_radii;

float sd_round_box(vec2 p, vec2 half_size, float radius) {
    float r = min(radius, min(half_size.x, half_size.y));
    vec2 q = abs(p) - half_size + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

void main() {
    vec4 color = texture2D(tex, v_coords);

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif

    vec2 px = v_coords * spectre_size;
    vec2 centre = (spectre_window_min + spectre_window_max) * 0.5;
    vec2 half_size = (spectre_window_max - spectre_window_min) * 0.5;
    vec2 p = px - centre;

    float radius = p.x < 0.0
        ? (p.y < 0.0 ? spectre_radii.x : spectre_radii.w)
        : (p.y < 0.0 ? spectre_radii.y : spectre_radii.z);

    const float AA = 0.8;
    float mask = 1.0 - smoothstep(-AA, AA, sd_round_box(p, half_size, radius));

    color = color * alpha * mask;

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
