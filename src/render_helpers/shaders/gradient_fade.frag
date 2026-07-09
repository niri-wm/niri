#version 100

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

// x is left edge, y is right edge of the gradient.
uniform vec2 cutoff;

void main() {
    // Sample the texture.
    vec4 color = texture2D(tex, v_coords);
#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif

    if (cutoff.x < cutoff.y) {
        float fade = clamp((cutoff.y - v_coords.x) / (cutoff.y - cutoff.x), 0.0, 1.0);
        color = color * fade;
    }

    // Apply final alpha and tint.
    color = color * alpha;

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
