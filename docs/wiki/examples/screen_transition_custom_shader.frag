// Your shader must contain one function (see the bottom of this file).
//
// It should not contain any uniform definitions or anything else, as niri
// provides them for you.
//
// All symbols defined by niri will have a niri_ prefix, so don't use it for
// your own variables and functions.

// The function that you must define looks like this:
vec4 screen_transition_color(vec3 coords_geo, vec3 size_geo) {
    vec4 color = /* ...compute the color... */;
    return color;
}

// It takes as input:
//
// * coords_geo: coordinates of the current pixel relative to the output.
//
// These are homogeneous (the Z component is equal to 1) and scaled in such a
// way that the 0 to 1 coordinates cover the full output.
//
// * size_geo: size of the output in logical pixels.
//
// It is homogeneous (the Z component is equal to 1).
//
// The screen transition shader renders as an overlay above the live workspace.
// Making pixels transparent (alpha = 0) will reveal the new workspace
// underneath, while opaque pixels (alpha = 1) will show the captured old
// workspace snapshot. As the transition progresses, you should generally make
// more pixels transparent to reveal the new workspace.
//
// The function must return the color of the pixel (with premultiplied alpha).
// The pixel color will be further processed by niri (e.g. to apply final
// alpha).

// Now let's go over the uniforms that niri defines.
//
// You should only rely on the uniforms documented here. Any other uniforms can
// change or be removed without notice.

// The captured texture of the old workspace (before the switch).
uniform sampler2D niri_tex_from;

// Matrix that converts geometry coordinates into the old workspace texture
// coordinates.
//
// You must always use this matrix to sample the texture, like this:
//   vec3 coords_tex = niri_geo_to_tex * coords_geo;
//   vec4 color = texture2D(niri_tex_from, coords_tex.st);
// This ensures correct sampling when the output has a transform (rotation).
uniform mat3 niri_geo_to_tex;


// Unclamped progress of the animation.
//
// Goes from 0 to 1 but may overshoot and oscillate.
uniform float niri_progress;

// Clamped progress of the animation.
//
// Goes from 0 to 1, but will stop at 1 as soon as it first reaches 1. Will not
// overshoot or oscillate.
uniform float niri_clamped_progress;

// Random float in [0; 1), consistent for the duration of the animation.
uniform float niri_random_seed;

// Mouse position in logical output coordinates (0,0 at top-left).
// Set to (-1, -1) if the cursor is not on this output.
uniform vec2 niri_mouse_pos;

// Now let's look at some examples. You can copy everything below this line
// into your custom-shader to experiment.

// Example: gradually fade out the old workspace, equivalent to the default
// crossfade transition.
vec4 default_crossfade(vec3 coords_geo, vec3 size_geo) {
    vec3 coords_tex = niri_geo_to_tex * coords_geo;
    vec4 color = texture2D(niri_tex_from, coords_tex.st);

    // Fade out the old workspace to reveal the new one underneath.
    color *= (1.0 - niri_clamped_progress);

    return color;
}

// Example: horizontal wipe from left to right revealing the new workspace.
vec4 horizontal_wipe(vec3 coords_geo, vec3 size_geo) {
    vec3 coords_tex = niri_geo_to_tex * coords_geo;
    vec4 color = texture2D(niri_tex_from, coords_tex.st);

    // The wipe edge moves from left (x=0) to right (x=1) with progress.
    // Pixels to the left of the edge become transparent, revealing the
    // new workspace underneath.
    float alpha = smoothstep(niri_clamped_progress - 0.05,
                             niri_clamped_progress + 0.05,
                             coords_geo.x);

    return color * alpha;
}

// Example: the old workspace slides upward off the screen.
vec4 slide_up(vec3 coords_geo, vec3 size_geo) {
    // Shift the sampling position upward as the transition progresses.
    vec3 shifted = vec3(coords_geo.x,
                        coords_geo.y + niri_clamped_progress,
                        1.0);

    // Pixels that have shifted beyond the old workspace should be
    // transparent, letting the new workspace show through.
    if (shifted.y > 1.0)
        return vec4(0.0);

    vec3 coords_tex = niri_geo_to_tex * shifted;
    vec4 color = texture2D(niri_tex_from, coords_tex.st);

    return color;
}

// Example: pixels randomly dissolve to reveal the new workspace.
vec4 pixel_dissolve(vec3 coords_geo, vec3 size_geo) {
    vec3 coords_tex = niri_geo_to_tex * coords_geo;
    vec4 color = texture2D(niri_tex_from, coords_tex.st);

    // Generate a pseudo-random value per pixel, offset by the random seed
    // so each transition looks different.
    vec2 seed = coords_geo.xy + vec2(niri_random_seed);
    float random = fract(sin(dot(seed, vec2(12.9898, 78.233))) * 43758.5453);

    // As progress increases, more pixels cross the threshold and become
    // transparent, revealing the new workspace.
    float threshold = niri_clamped_progress * 1.2;
    float alpha = step(threshold, random);

    return color * alpha;
}

// Example: expanding circle from mouse position revealing the new workspace.
// Recommended setting: duration-ms 300
vec4 circle_reveal(vec3 coords_geo, vec3 size_geo) {
    vec3 coords_tex = niri_geo_to_tex * coords_geo;
    vec4 color = texture2D(niri_tex_from, coords_tex.st);

    // Use the mouse position as the circle center, or the screen center
    // if the cursor is off this output (sentinel value -1, -1).
    vec2 center = (niri_mouse_pos.x < 0.0)
        ? vec2(0.5)
        : niri_mouse_pos / size_geo.xy;

    // Correct for aspect ratio so the reveal is a circle, not an ellipse.
    vec2 aspect = size_geo.xy / length(size_geo.xy);
    vec2 delta = (coords_geo.xy - center) * aspect;
    float dist = length(delta);

    // The circle expands with progress. Scale by ~1.5 to ensure
    // it covers the full screen even when starting from a corner.
    float radius = niri_clamped_progress * 1.5;

    // Pixels inside the expanding circle become transparent (new workspace),
    // pixels outside stay opaque (old workspace).
    float alpha = smoothstep(radius - 0.05, radius, dist);

    return color * alpha;
}

// This is the function that you must define.
vec4 screen_transition_color(vec3 coords_geo, vec3 size_geo) {
    // You can pick one of the example functions or write your own.
    return circle_reveal(coords_geo, size_geo);
}
