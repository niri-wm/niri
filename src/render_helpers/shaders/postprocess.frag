uniform float noise;
uniform float saturation;
uniform vec4 bg_color;

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
