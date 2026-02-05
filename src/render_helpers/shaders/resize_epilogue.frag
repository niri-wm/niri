
void main() {
    vec3 coords_curr_geo = niri_input_to_curr_geo * vec3(niri_v_coords, 1.0);
    vec3 size_curr_geo = vec3(niri_curr_geo_size, 1.0);

    vec4 color = resize_color(coords_curr_geo, size_curr_geo);

    if (niri_clip_to_geometry == 1.0) {
        if (coords_curr_geo.x < 0.0 || 1.0 < coords_curr_geo.x
                || coords_curr_geo.y < 0.0 || 1.0 < coords_curr_geo.y) {
            // Clip outside geometry.
            color = vec4(0.0);
        } else {
            // Apply corner rounding inside geometry.
            color = color * niri_rounding_alpha(coords_curr_geo.xy * size_curr_geo.xy, size_curr_geo.xy, niri_corner_radius);
        }
    }

    color = color * niri_alpha;

#if defined(DEBUG_FLAGS)
    if (niri_tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
