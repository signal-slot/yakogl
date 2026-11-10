#version 100

struct Fade
{
    mat4 matrix;
    vec4 offsets;
    vec3 scale;
    float opacity;
};

uniform Fade u_fade;

varying float v_mix;
varying vec4 v_uv2;

attribute vec2 a_uv;
attribute vec2 a_pos;

void main()
{
    v_mix = u_fade.scale.z;
    v_uv2 = vec4(u_fade.offsets.xy + a_uv * u_fade.scale.xy, u_fade.offsets.zw + a_uv * u_fade.scale.xy);
    gl_Position = u_fade.matrix * vec4(a_pos, 0.0, 1.0);
}
