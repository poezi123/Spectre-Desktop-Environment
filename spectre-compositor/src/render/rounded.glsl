// Rounds the corners of a client surface.
//
// Installed as a texture shader override while one window's surfaces are drawn,
// so the window's own rectangle - not each surface's - decides where the
// corners are. A subsurface that reaches into a corner is therefore clipped by
// the same curve as the toplevel.

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

// Size of the element being drawn, in device pixels.
uniform vec2 spectre_size;
// The window's rectangle in element-local device pixels.
uniform vec2 spectre_window_min;
uniform vec2 spectre_window_max;
// Corner radii: top-left, top-right, bottom-right, bottom-left.
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

    // Pick the radius belonging to the quadrant this fragment is in, so the
    // top corners can stay square under a title bar while the bottom ones round.
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
