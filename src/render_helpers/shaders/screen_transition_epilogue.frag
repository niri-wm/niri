void main() {
    vec3 coords_input = vec3(niri_v_coords, 1.0);
    vec3 size_input = vec3(niri_size, 1.0);

    vec4 color = screen_transition_color(coords_input, size_input);

    color = color * niri_alpha;

#if defined(DEBUG_FLAGS)
    if (niri_tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
