precision highp float;

#if defined(DEBUG_FLAGS)
uniform float niri_tint;
#endif

varying vec2 niri_v_coords;
uniform vec2 niri_size;

uniform sampler2D niri_tex_from;
uniform mat3 niri_geo_to_tex;

uniform float niri_progress;
uniform float niri_clamped_progress;
uniform float niri_alpha;
uniform float niri_scale;

// Mouse position in logical output coordinates (0,0 at top-left).
uniform vec2 niri_mouse_pos;

// Random seed generated once per transition.
uniform float niri_random_seed;
