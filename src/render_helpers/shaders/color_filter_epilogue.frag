
void main() {
    vec4 color = texture2D(tex, v_coords);
#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif

    // Unpremultiply for user's color_filter.
    if (color.a > 0.0) {
        vec3 rgb = color.rgb / color.a;
        rgb = color_filter(rgb);
        color = vec4(rgb * color.a, color.a);
    }

    color = color * alpha;

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
