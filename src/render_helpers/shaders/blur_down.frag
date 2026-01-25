#version 100

precision highp float;

varying vec2 v_coords;

uniform sampler2D tex;
uniform vec2 half_pixel;
uniform float offset;

void main() {
    vec2 o = half_pixel * offset;

    vec4 sum = texture2D(tex, v_coords) * 4.0;
    sum += texture2D(tex, v_coords + vec2(-o.x, -o.y));
    sum += texture2D(tex, v_coords + vec2( o.x, -o.y));
    sum += texture2D(tex, v_coords + vec2(-o.x,  o.y));
    sum += texture2D(tex, v_coords + vec2( o.x,  o.y));

    gl_FragColor = sum / 8.0;
}
