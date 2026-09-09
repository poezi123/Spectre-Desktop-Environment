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

uniform vec4 spectre_line_0;
uniform vec4 spectre_line_1;
uniform vec4 spectre_line_2;
uniform vec4 spectre_line_3;
uniform float spectre_color_phase;
uniform float spectre_color_span;
uniform vec4 spectre_bg;
uniform float spectre_uv_origin;
uniform float spectre_uv_span;

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

void main() {
    float field = texture2D(tex, v_coords).a;

    float across = spectre_uv_span > 0.0
        ? (v_coords.x - spectre_uv_origin) / spectre_uv_span
        : v_coords.x;
    vec4 line_color = spectre_line_at(across * spectre_color_span + spectre_color_phase);

    float coverage = field * line_color.a;
    vec3 rgb = mix(spectre_ground(spectre_bg.rgb, line_color.rgb), line_color.rgb, coverage);
    float a = spectre_bg.a + (1.0 - spectre_bg.a) * coverage;

    vec4 color = vec4(rgb * a, a) * alpha;

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
