precision highp float;

#if defined(DEBUG_FLAGS)
uniform float niri_tint;
#endif

uniform float niri_alpha;
uniform float niri_scale;

uniform vec2 niri_size;
varying vec2 niri_v_coords;

// Premultiplied alpha.
uniform vec4 color;
uniform float dwell_progress;
uniform float clicked_progress;
uniform float stroke_width;
uniform float outer_radius;

const float PI = 3.14159265358979323846;
const float TWO_PI = 6.28318530717958647692;

void main() {
    vec2 p = (niri_v_coords - 0.5) * niri_size;
    float r = length(p);

    float stroke = stroke_width * niri_scale;
    float base_r = outer_radius * niri_scale;
    float current_outer_r = base_r * (1.0 - clamp(clicked_progress, 0.0, 1.0));
    if (current_outer_r <= 0.0) {
        gl_FragColor = vec4(0.0);
        return;
    }

    float half_w = min(stroke * 0.5, current_outer_r * 0.5);
    float ring_r = max(current_outer_r - half_w, 0.0);

    float d_radial = abs(r - ring_r) - half_w;
    float alpha_radial =
        (1.0 - smoothstep(-0.5, 0.5, d_radial)) * clamp(current_outer_r, 0.0, 1.0);

    float progress = clamp(dwell_progress, 0.0, 1.0);
    float alpha_arc = 0.0;
    if (progress >= 1.0) {
        alpha_arc = 1.0;
    } else if (progress > 0.0 && r >= 0.001) {
        // Angle clockwise from 12 o'clock in [0, TWO_PI).
        float angle = atan(p.x, -p.y);
        if (angle < 0.0) {
            angle += TWO_PI;
        }

        float half_sweep = progress * PI;
        float delta = mod(angle - half_sweep + PI, TWO_PI) - PI;
        float arc_dist = (half_sweep - abs(delta)) * r;
        float arc_len = half_sweep * r * 2.0;
        float gap_len = (PI - half_sweep) * r * 2.0;
        alpha_arc = smoothstep(-0.5, 0.5, arc_dist) * clamp(arc_len, 0.0, 1.0);
        alpha_arc = max(alpha_arc, 1.0 - clamp(gap_len, 0.0, 1.0));
    }

    vec4 out_color = color * (alpha_radial * alpha_arc * niri_alpha);

#if defined(DEBUG_FLAGS)
    if (niri_tint == 1.0)
        out_color = vec4(0.0, 0.2, 0.0, 0.2) + out_color * 0.8;
#endif

    gl_FragColor = out_color;
}
