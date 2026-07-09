
void main() {
    vec3 coords_geo = niri_input_to_geo * vec3(niri_v_coords, 1.0);
    vec3 size_geo = vec3(niri_geo_size, 1.0);

    vec4 color = close_color(coords_geo, size_geo);

    color = color * niri_alpha;

#if defined(DEBUG_FLAGS)
    if (niri_tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
