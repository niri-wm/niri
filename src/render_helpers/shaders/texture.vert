#version 100

uniform mat3 matrix;
uniform mat3 tex_matrix;

attribute vec2 vert;
attribute vec4 vert_position;

varying vec2 niri_v_coords;

mat2 scale(vec2 scale_vec){
    return mat2(
        scale_vec.x, 0.0,
        0.0, scale_vec.y
    );
}

void main() {
    vec2 vert_transform_translation = vert_position.xy;
    vec2 vert_transform_scale = vert_position.zw;
    vec3 position = vec3(vert * scale(vert_transform_scale) + vert_transform_translation, 1.0);
    niri_v_coords = (tex_matrix * position).xy;
    gl_Position = vec4(matrix * position, 1.0);
}

