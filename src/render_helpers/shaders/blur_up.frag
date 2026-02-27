#version 100

precision highp float;

varying vec2 v_coords;

uniform sampler2D tex;
uniform vec2 half_pixel;
uniform float offset;

void main() {
    vec2 o = half_pixel * offset;

    vec4 sum = vec4(0.0);

    // Four edge centers
    sum += texture2D(tex, v_coords + vec2(-o.x * 2.0, 0.0));
    sum += texture2D(tex, v_coords + vec2( o.x * 2.0, 0.0));
    sum += texture2D(tex, v_coords + vec2(0.0, -o.y * 2.0));
    sum += texture2D(tex, v_coords + vec2(0.0,  o.y * 2.0));

    // Four diagonal corners
    sum += texture2D(tex, v_coords + vec2(-o.x,  o.y)) * 2.0;
    sum += texture2D(tex, v_coords + vec2( o.x,  o.y)) * 2.0;
    sum += texture2D(tex, v_coords + vec2(-o.x, -o.y)) * 2.0;
    sum += texture2D(tex, v_coords + vec2( o.x, -o.y)) * 2.0;

    gl_FragColor = sum / 12.0;
}
