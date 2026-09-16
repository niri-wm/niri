#version 100

//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

uniform float niri_scale;

uniform vec2 geo_size;
uniform vec4 corner_radius;
uniform mat3 input_to_geo;
uniform vec4 refraction_params;
uniform vec2 effect_params;

float niri_rounding_alpha(vec2 coords, vec2 size, vec4 corner_radius);
float niri_rounded_box_sdf(vec2 p, vec2 size, vec4 corner_radius);
vec2 niri_rounded_box_normal(vec2 p, vec2 size, vec4 corner_radius);
vec4 postprocess(vec4 color);

vec2 geo_to_input_delta(vec2 delta_px, vec2 size) {
    // input_to_geo maps normalized source coordinates to normalized geometry
    // coordinates. Convert a geometry-pixel delta through the inverse linear
    // part before applying it to the source texture coordinate.
    float a = input_to_geo[0][0];
    float b = input_to_geo[1][0];
    float c = input_to_geo[0][1];
    float d = input_to_geo[1][1];
    // GLSL indexing is [column][row], so the linear rows are [a, b]
    // and [c, d] for the values above.
    float det = a * d - b * c;
    if (abs(det) < 0.000001) {
        return delta_px / size;
    }

    vec2 delta = delta_px / size;
    return vec2(
        (d * delta.x - b * delta.y) / det,
        (-c * delta.x + a * delta.y) / det
    );
}

void main() {
    float refraction = refraction_params.x;
    float refraction_bevel = refraction_params.y;
    float refraction_saturation = refraction_params.z;
    float refraction_brightness = refraction_params.w;
    float feather = effect_params.x;
    float dim = effect_params.y;

    vec3 coords_geo = input_to_geo * vec3(v_coords, 1.0);
    vec2 sample_coords = v_coords;
    float specular = 0.0;

    // Compute the rounded-rectangle SDF once and reuse it for refraction and feather.
    // Refraction needs a sane size for stable bevel math; feather works at any size.
    bool big_enough = geo_size.x > 2.0 && geo_size.y > 2.0;
    bool do_refr = refraction > 0.001 && big_enough;
    bool do_feather = feather > 0.001;
    bool need_sdf = do_refr || do_feather;

    // Shared rounded-rect SDF state (valid only when need_sdf).
    float sdf_d = 0.0;
    vec2 sdf_p = vec2(0.0);

    if (need_sdf) {
        vec2 px = coords_geo.xy * geo_size;
        vec2 p = px - geo_size * 0.5;
        sdf_d = niri_rounded_box_sdf(p, geo_size, corner_radius);
        sdf_p = p;
    }

    // Optical Snell's law refraction along curved surface bevel.
    if (do_refr) {
        // Bevel width: explicit if configured (> 0.001), otherwise scaled with corner radius.
        float r_eff = max(max(corner_radius.x, corner_radius.y),
            max(corner_radius.z, corner_radius.w));
        r_eff = max(r_eff, 1.0);
        float min_dim = min(geo_size.x, geo_size.y);
        float bevel_auto_max = min_dim * 0.35;
        float bevel_auto_min = min(8.0, bevel_auto_max);
        float bevel_auto = clamp(r_eff * 0.85, bevel_auto_min, bevel_auto_max);
        float bevel_max = min_dim * 0.45;
        float bevel_min = min(1.0, bevel_max);
        float bevel = (refraction_bevel > 0.001) ? clamp(refraction_bevel, bevel_min, bevel_max) : bevel_auto;

        // Evaluate refraction within the outer curved bevel strictly inside the geometry.
        if (bevel >= 0.5 && sdf_d >= -bevel && sdf_d <= 0.0) {
            // Surface normal from the asymmetric rounded-rectangle SDF.
            vec2 dir = niri_rounded_box_normal(sdf_p, geo_size, corner_radius);
            float edge_dist = -sdf_d;
            float u = edge_dist / bevel;

            // Sextic shoulder matching the superellipse profile (slope 3.5 at
            // boundary, vanishing inward to keep the center plate flat).
            float inv = 1.0 - u;
            float inv2 = inv * inv;
            float inv6 = inv2 * inv2 * inv2;
            float slope = min(3.5 * inv6, 3.5);

            // Smooth boundary feathering:
            // Vanishes to 0 within the outermost pixel (u <= 0.04) so edge pixels never
            // reach inward to pull light streaks onto the boundary (eliminating white fringes).
            // Smoothly ramps to 1.0 across the bevel shoulder (u = 0.16) for full refraction.
            float refr_feather = smoothstep(0.04, 0.16, u);

            // Convex 3D surface normal.
            vec3 normal = normalize(vec3(dir * (slope * refr_feather * 0.75), 1.0));

            // Snell's law vector refraction (eta = 1.0 / 2.2).
            // With eta < 1 and normal.z >= ~0.35, total internal reflection cannot occur.
            vec3 view_dir = vec3(0.0, 0.0, -1.0);
            float eta = 1.0 / 2.2;
            vec3 refr_ray = refract(view_dir, normal, eta);

            float safe_z = max(-refr_ray.z, 0.18);
            float disp_scale = refraction * 16.0;
            vec2 disp_px = (refr_ray.xy / safe_z) * disp_scale;
            // Clamp displacement in geometry space before converting it to source
            // coordinates, then keep the final texture lookup inside the source.
            vec2 max_px = vec2(min(bevel * 1.8, min_dim * 0.4));
            disp_px = clamp(disp_px, -max_px, max_px);
            vec2 disp_uv = geo_to_input_delta(disp_px, geo_size);
            sample_coords = clamp(v_coords + disp_uv, vec2(0.001), vec2(0.999));

            // Directional specular glint from overhead light (-0.33, -0.94).
            vec2 light_source = vec2(-0.330, -0.944);
            float light_dot = clamp(dot(dir, light_source), 0.0, 1.0);
            float directional_sheen = 0.20 + 0.80 * light_dot;

            // Meniscus glint on the bevel shoulder, fading to zero at the outer boundary.
            float sheen_profile = refr_feather * inv * inv * inv;
            specular = sheen_profile * 0.40 * clamp(refraction, 0.0, 1.0) * directional_sheen;
        }
    }

    // Sample the background texture.
    vec4 color = texture2D(tex, sample_coords);
#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif

    // Radiance and saturation boost for refracted surfaces. The texture is
    // premultiplied, so tune straight color and re-premultiply afterwards.
    if (refraction > 0.001) {
        if (color.a > 0.0001) {
            vec3 straight = color.rgb / color.a;
            float luma = dot(straight, vec3(0.2126, 0.7152, 0.0722));
            straight = mix(vec3(luma), straight, refraction_saturation);
            straight = clamp(straight * refraction_brightness, 0.0, 1.0);
            color.rgb = straight * color.a;
        } else {
            color.rgb = vec3(0.0);
        }
    }

    color = postprocess(color);
    color.rgb = clamp(color.rgb, vec3(0.0), vec3(max(color.a, 0.0)));

    // Specular rim reflection, also in premultiplied-alpha space.
    if (specular > 0.0005) {
        color.rgb += (vec3(color.a) - color.rgb) * specular;
    }

    // Dimming applied in the same coordinate space.
    if (dim > 0.001) {
        color.rgb = mix(color.rgb, vec3(0.0), clamp(dim, 0.0, 1.0));
    }

    if (!do_feather) {
        if (coords_geo.x < 0.0 || 1.0 < coords_geo.x || coords_geo.y < 0.0 || 1.0 < coords_geo.y) {
            // Clip outside geometry.
            color = vec4(0.0);
        } else {
            // Apply corner rounding inside geometry.
            color = color * niri_rounding_alpha(coords_geo.xy * geo_size, geo_size, corner_radius);
        }
    } else {
        // Reuse shared SDF; no second evaluation.
        float d = sdf_d;

        if (coords_geo.x < 0.0 || 1.0 < coords_geo.x || coords_geo.y < 0.0 || 1.0 < coords_geo.y || d >= 0.0) {
            // Clip outside geometry.
            color = vec4(0.0);
        } else {
            // Progressive smooth falloff from inner core (d <= -feather, factor = 1.0)
            // to outer boundary (d == 0.0, factor = 0.0).
            float factor = 1.0;
            if (d > -feather) {
                float t = clamp(-d / feather, 0.0, 1.0);
                // Cubic-in: holds ~full strength until close to the edge, so a
                // wide ramp softens sides without eating interior content.
                // (Shared curve with shadow feather — keep the two in sync.)
                float u = 1.0 - t;
                factor = 1.0 - u * u * u;
            }

            // Preserve the same pixel-coverage antialiasing as the hard-edge path.
            float coverage = niri_rounding_alpha(
                coords_geo.xy * geo_size,
                geo_size,
                corner_radius
            );
            color = color * factor * coverage;
        }
    }

    // Apply final alpha and tint.
    color = color * alpha;

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
