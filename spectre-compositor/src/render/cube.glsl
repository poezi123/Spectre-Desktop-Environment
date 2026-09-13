//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision highp float;

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

uniform float spectre_angle;
uniform float spectre_apothem;
uniform float spectre_camera;
uniform float spectre_scale;
uniform float spectre_aspect;
uniform float spectre_flip;

void main() {
    vec2 screen = vec2(
        (v_coords.x - 0.5) / spectre_scale,
        (0.5 - v_coords.y) * spectre_aspect / spectre_scale
    );
    vec3 eye = vec3(0.0, 0.0, spectre_camera);
    vec3 ray = vec3(screen, spectre_apothem) - eye;

    vec3 normal = vec3(sin(spectre_angle), 0.0, cos(spectre_angle));
    vec3 across = vec3(cos(spectre_angle), 0.0, -sin(spectre_angle));
    float facing = dot(ray, normal);
    if (facing >= 0.0) {
        discard;
    }

    vec3 center = normal * spectre_apothem;
    float distance = dot(center - eye, normal) / facing;
    vec3 local = eye + ray * distance - center;

    float u = dot(local, across) + 0.5;
    float v = 0.5 - local.y / spectre_aspect;
    if (u < 0.0 || u > 1.0 || v < 0.0 || v > 1.0) {
        discard;
    }
    if (spectre_flip > 0.5) {
        v = 1.0 - v;
    }

    vec4 color = texture2D(tex, vec2(u, v));
    float light = 0.45 + 0.55 * max(normal.z, 0.0);
    color = vec4(color.rgb * light, color.a) * alpha;

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
