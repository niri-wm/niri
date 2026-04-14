uniform vec4 bg_color;

vec4 postprocess(vec4 color) {
    // Mix bg_color behind the texture (both premultiplied alpha).
    color = color + bg_color * (1.0 - color.a);

    return color;
}
