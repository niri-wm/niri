// Signed distance and finite-difference normal for a rounded rectangle with
// independent corner radii. Corner order is top-left, top-right,
// bottom-right, bottom-left, matching niri_config::CornerRadius.
float niri_sd_segment(vec2 p, vec2 a, vec2 b) {
    vec2 pa = p - a;
    vec2 ba = b - a;
    float h = clamp(dot(pa, ba) / max(dot(ba, ba), 0.000001), 0.0, 1.0);
    return length(pa - ba * h);
}

float niri_sd_quarter_arc(vec2 p, vec2 center, float radius, vec2 direction) {
    vec2 q = p - center;
    bool in_quadrant = (direction.x < 0.0 ? q.x <= 0.0 : q.x >= 0.0)
        && (direction.y < 0.0 ? q.y <= 0.0 : q.y >= 0.0);

    if (in_quadrant) {
        return abs(length(q) - radius);
    }

    vec2 endpoint_x = vec2(center.x + direction.x * radius, center.y);
    vec2 endpoint_y = vec2(center.x, center.y + direction.y * radius);
    return min(length(p - endpoint_x), length(p - endpoint_y));
}

float niri_rounded_box_sdf(vec2 p, vec2 size, vec4 corner_radius) {
    float max_radius = min(size.x, size.y) * 0.5;
    vec4 r = clamp(corner_radius, 0.0, max_radius);
    vec2 half_size = size * 0.5;
    vec2 top_left = p + half_size;

    vec2 tl = vec2(r.x, r.x);
    vec2 tr = vec2(size.x - r.y, r.y);
    vec2 br = vec2(size.x - r.z, size.y - r.z);
    vec2 bl = vec2(r.w, size.y - r.w);

    float d = niri_sd_segment(top_left, vec2(r.x, 0.0), vec2(size.x - r.y, 0.0));
    d = min(d, niri_sd_segment(top_left, vec2(size.x, r.y), vec2(size.x, size.y - r.z)));
    d = min(d, niri_sd_segment(top_left, vec2(size.x - r.z, size.y), vec2(r.w, size.y)));
    d = min(d, niri_sd_segment(top_left, vec2(0.0, size.y - r.w), vec2(0.0, r.x)));
    d = min(d, niri_sd_quarter_arc(top_left, tl, r.x, vec2(-1.0, -1.0)));
    d = min(d, niri_sd_quarter_arc(top_left, tr, r.y, vec2(1.0, -1.0)));
    d = min(d, niri_sd_quarter_arc(top_left, br, r.z, vec2(1.0, 1.0)));
    d = min(d, niri_sd_quarter_arc(top_left, bl, r.w, vec2(-1.0, 1.0)));

    bool inside = top_left.x >= 0.0 && top_left.x <= size.x
        && top_left.y >= 0.0 && top_left.y <= size.y;
    if (top_left.x < r.x && top_left.y < r.x
        && length(top_left - tl) > r.x) {
        inside = false;
    }
    if (size.x - top_left.x < r.y && top_left.y < r.y
        && length(top_left - tr) > r.y) {
        inside = false;
    }
    if (size.x - top_left.x < r.z && size.y - top_left.y < r.z
        && length(top_left - br) > r.z) {
        inside = false;
    }
    if (top_left.x < r.w && size.y - top_left.y < r.w
        && length(top_left - bl) > r.w) {
        inside = false;
    }

    return inside ? -d : d;
}

vec2 niri_rounded_box_normal(vec2 p, vec2 size, vec4 corner_radius) {
    float eps = 0.25;
    float dx = niri_rounded_box_sdf(p + vec2(eps, 0.0), size, corner_radius)
        - niri_rounded_box_sdf(p - vec2(eps, 0.0), size, corner_radius);
    float dy = niri_rounded_box_sdf(p + vec2(0.0, eps), size, corner_radius)
        - niri_rounded_box_sdf(p - vec2(0.0, eps), size, corner_radius);
    vec2 normal = vec2(dx, dy);
    float len = length(normal);
    if (len < 0.0001) {
        return vec2(0.0, 1.0);
    }
    return normal / len;
}
