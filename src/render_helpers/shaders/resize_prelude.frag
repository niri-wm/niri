precision highp float;

#if defined(DEBUG_FLAGS)
uniform float niri_tint;
#endif

varying vec2 niri_v_coords;
uniform vec2 niri_size;

uniform mat3 niri_input_to_curr_geo;
uniform mat3 niri_curr_geo_to_prev_geo;
uniform mat3 niri_curr_geo_to_next_geo;
uniform vec2 niri_curr_geo_size;

uniform sampler2D niri_tex_prev;
uniform mat3 niri_geo_to_tex_prev;

uniform sampler2D niri_tex_next;
uniform mat3 niri_geo_to_tex_next;

uniform float niri_progress;
uniform float niri_clamped_progress;

uniform vec4 niri_corner_radius;
uniform float niri_clip_to_geometry;

uniform float niri_alpha;
uniform float niri_scale;

float niri_rounding_alpha(vec2 coords, vec2 size, vec4 corner_radius);
