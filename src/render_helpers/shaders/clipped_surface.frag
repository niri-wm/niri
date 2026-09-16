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
uniform float refraction;
uniform float refraction_bevel;
uniform float feather;
uniform float dim;
uniform float refraction_saturation;
uniform float refraction_brightness;

float niri_rounding_alpha(vec2 coords, vec2 size, vec4 corner_radius);
vec4 postprocess(vec4 color);

void main() {
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
    float sdf_r = 0.0;
    float sdf_d = 0.0;
    vec2 sdf_p = vec2(0.0);
    vec2 sdf_q = vec2(0.0);

    if (need_sdf) {
        vec2 px = coords_geo.xy * geo_size;
        vec2 b = geo_size * 0.5;
        vec2 p = px - b;

        // Quadrant corner radius, clamped to half-extents.
        // Canonical radius uses 0-clamp (correct for square corners); the refraction
        // bevel derives a >=1px effective radius from it below.
        float r_raw = (coords_geo.x < 0.5)
            ? ((coords_geo.y < 0.5) ? corner_radius.x : corner_radius.w)
            : ((coords_geo.y < 0.5) ? corner_radius.y : corner_radius.z);
        float r = clamp(r_raw, 0.0, min(b.x, b.y));

        // Signed distance to rounded rectangle.
        vec2 q = abs(p) - b + vec2(r);
        float d = min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0))) - r;

        sdf_r = r;
        sdf_d = d;
        sdf_p = p;
        sdf_q = q;
    }

    // Optical Snell's law refraction along curved surface bevel.
    if (do_refr) {
        // Analytic surface normal direction vector.
        vec2 dir;
        if (max(sdf_q.x, sdf_q.y) < 0.0) {
            dir = (sdf_q.x > sdf_q.y) ? vec2(sign(sdf_p.x), 0.0) : vec2(0.0, sign(sdf_p.y));
        } else {
            vec2 qc = max(sdf_q, vec2(0.0));
            dir = sign(sdf_p) * (qc / max(length(qc), 1e-4));
        }

        // Bevel width: explicit if configured (> 0.001), otherwise scaled with corner radius.
        float r_eff = max(sdf_r, 1.0);
        float min_dim = min(geo_size.x, geo_size.y);
        float bevel_auto = clamp(r_eff * 0.85, 8.0, min_dim * 0.35);
        float bevel = (refraction_bevel > 0.001) ? clamp(refraction_bevel, 1.0, min_dim * 0.45) : bevel_auto;

        // Evaluate refraction within the outer curved bevel strictly inside the geometry.
        if (bevel >= 0.5 && sdf_d >= -bevel && sdf_d <= 0.0) {
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
            vec2 disp_uv = disp_px / geo_size;
            // Clamp displacement to prevent sampling far outside the bevel neighborhood.
            vec2 max_uv = vec2(min(bevel * 1.8, min_dim * 0.4)) / geo_size;
            disp_uv = clamp(disp_uv, -max_uv, max_uv);
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

    // Radiance and saturation boost for refracted surfaces.
    if (refraction > 0.001) {
        float luma = dot(color.rgb, vec3(0.2126, 0.7152, 0.0722));
        color.rgb = mix(vec3(luma), color.rgb, refraction_saturation);
        color.rgb = clamp(color.rgb * refraction_brightness, 0.0, 1.0);
    }

    color = postprocess(color);

    // Specular rim reflection.
    if (specular > 0.0005) {
        color.rgb += (vec3(1.0) - color.rgb) * specular;
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
        } else if (d > -feather) {
            // Progressive smooth falloff from inner core (d <= -feather, factor = 1.0)
            // to outer boundary (d == 0.0, factor = 0.0).
            float t = clamp(-d / feather, 0.0, 1.0);
            // Cubic-in: holds ~full strength until close to the edge, so a
            // wide ramp softens sides without eating interior content.
            // (Shared curve with shadow feather — keep the two in sync.)
            float u = 1.0 - t;
            float factor = 1.0 - u * u * u;
            color = color * factor;
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
