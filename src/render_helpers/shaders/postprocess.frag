uniform float noise;
uniform float saturation;
uniform vec4 bg_color;

uniform sampler2D surface_tex;
uniform int alpha_mask_enabled;
uniform float alpha_threshold;
uniform mat3 surface_to_geo;

// Sin-less white noise by David Hoskins (MIT License).
// https://www.shadertoy.com/view/4djSRW
float hash12(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

vec3 saturate(vec3 color, float sat) {
    const vec3 w = vec3(0.2126, 0.7152, 0.0722);
    return mix(vec3(dot(color, w)), color, sat);
}

vec4 postprocess(vec4 color) {
    // Alpha-mask the blurred backdrop by the layer surface's own alpha. Fragments where the
    // surface is fully transparent get zeroed so the unblurred backdrop shows through.
    if (alpha_mask_enabled == 1) {
        vec3 surface_coords = surface_to_geo * vec3(v_coords, 1.0);
        if (surface_coords.x < 0.0 || surface_coords.x > 1.0
            || surface_coords.y < 0.0 || surface_coords.y > 1.0) {
            return vec4(0.0);
        }
        float surface_a = texture2D(surface_tex, surface_coords.xy).a;
        if (surface_a <= alpha_threshold) {
            return vec4(0.0);
        }
    }

    if (saturation != 1.0) {
        color.rgb = saturate(color.rgb, saturation);
    }

    if (noise > 0.0) {
        vec2 uv = gl_FragCoord.xy;
        color.rgb += (hash12(uv) - 0.5) * noise;
    }

    // Mix bg_color behind the texture (both premultiplied alpha).
    color = color + bg_color * (1.0 - color.a);

    return color;
}
