float niri_superellipse_dist(vec2 coords, vec2 center, float radius, float exponent) {
    vec2 delta = abs(coords - center);

    if (abs(exponent - 2.0) < 0.001) {
        return length(delta) - radius;
    }

    if (abs(exponent - 1.0) < 0.001) {
        return delta.x + delta.y - radius;
    }

    vec2 normalized = delta / max(radius, 0.0001);
    float lp_norm = pow(pow(normalized.x, exponent) + pow(normalized.y, exponent), 1.0 / exponent);
    return (lp_norm - 1.0) * radius;
}

float niri_rounding_alpha_impl(vec2 coords, vec2 center, float radius, float corner_exponent) {
    float exponent = max(corner_exponent, 0.01);
    float dist = niri_superellipse_dist(coords, center, radius, exponent);
    float half_px = 0.5 / niri_scale;
    return 1.0 - smoothstep(-half_px, half_px, dist);
}
