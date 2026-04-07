// Your shader must contain one function (see the bottom of this file).
//
// It should not contain any uniform definitions or anything else, as niri
// provides them for you.
//
// All symbols defined by niri will have a niri_ prefix, so don't use it for
// your own variables and functions.

// The function that you must define looks like this:
vec4 close_color(vec3 coords_geo, vec3 size_geo) {
    vec4 color = /* ...compute the color... */;
    return color;
}

// It takes as input:
//
// * coords_geo: coordinates of the current pixel relative to the window
//   geometry.
//
// These are homogeneous (the Z component is equal to 1) and scaled in such a
// way that the 0 to 1 coordinates lie within the window geometry. Pixels
// outside the window geometry will have coordinates below 0 or above 1.
//
// The window geometry is its "visible bounds" from the user's perspective.
//
// The shader runs over the full screen area, so you must expect and handle
// coordinates outside the [0, 1] range. If the window is scrolled off-screen,
// all of the coordinates to the shader can fall outside the [0, 1] range.
//
// * size_geo: size of the window geometry in logical pixels.
//
// It is homogeneous (the Z component is equal to 1).
//
// The function must return the color of the pixel (with premultiplied alpha).
// The pixel color will be further processed by niri (for example, to apply the
// final opacity from window rules).

// Now let's go over the uniforms that niri defines.
//
// You should only rely on the uniforms documented here. Any other uniforms can
// change or be removed without notice.

// The window texture.
uniform sampler2D niri_tex;

// Matrix that converts geometry coordinates into the window texture
// coordinates.
//
// The window texture can and will go outside the geometry (for client-side
// decoration shadows for example), which is why this matrix is necessary.
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

// Now let's look at some examples. You can copy everything below this line
// into your custom-shader to experiment.

// Example: fill the current geometry with a solid vertical gradient and
// gradually make transparent.
vec4 solid_gradient(vec3 coords_geo, vec3 size_geo) {
    vec4 color = vec4(0.0);

    // Paint only the area inside the current geometry.
    if (0.0 <= coords_geo.x && coords_geo.x <= 1.0
            && 0.0 <= coords_geo.y && coords_geo.y <= 1.0)
    {
        vec4 from = vec4(1.0, 0.0, 0.0, 1.0);
        vec4 to = vec4(0.0, 1.0, 0.0, 1.0);
        color = mix(from, to, coords_geo.y);
    }

    // Make it transparent.
    color *= (1.0 - niri_clamped_progress);

    return color;
}

// Example: gradually scale down and make transparent, equivalent to the
// default closing animation.
vec4 default_close(vec3 coords_geo, vec3 size_geo) {
    // Scale down the window.
    float scale = max(0.0, ((1.0 - niri_clamped_progress) / 5.0 + 0.8));
    coords_geo = vec3((coords_geo.xy - vec2(0.5)) / scale + vec2(0.5), 1.0);

    // Get color from the window texture.
    vec3 coords_tex = niri_geo_to_tex * coords_geo;
    vec4 color = texture2D(niri_tex, coords_tex.st);

    // Make the window transparent.
    color *= (1.0 - niri_clamped_progress);

    return color;
}

// Example: make the window 'fall down' with slight rotation.
vec4 fall_and_rotate(vec3 coords_geo, vec3 size_geo) {
    // For this shader, set animation curve to linear for best results.

    // Simulate an accelerated fall: square the (linear) progress.
    float progress = niri_clamped_progress * niri_clamped_progress;

    // Get our rotation pivot point coordinates at the bottom center of the window.
    vec2 coords = (coords_geo.xy - vec2(0.5, 1.0)) * size_geo.xy;

    // Move the window down to simulate a fall.
    coords.y -= progress * 200.0;

    // Randomize rotation direction and maximum angle.
    float random = (niri_random_seed - 0.5) / 2.0;
    random = sign(random) - random;
    float max_angle = 0.05 * random;

    // Rotate the window around our pivot point.
    float angle = progress * max_angle;
    mat2 rotate = mat2(cos(angle), -sin(angle), sin(angle), cos(angle));
    coords = rotate * coords;

    // Transform the coordinates back.
    coords_geo = vec3(coords / size_geo.xy + vec2(0.5, 1.0), 1.0);

    // Sample the window texture.
    vec3 coords_tex = niri_geo_to_tex * coords_geo;
    vec4 color = texture2D(niri_tex, coords_tex.st);

    // Multiply by alpha to fade out.
    return color * (1.0 - niri_clamped_progress);
}

// This is the function that you must define.
vec4 close_color(vec3 coords_geo, vec3 size_geo) {
    // You can pick one of the example functions or write your own.
    return fall_and_rotate(coords_geo, size_geo);
}

