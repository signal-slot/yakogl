#version 100
precision highp float;

struct Fade
{
    mat4 matrix;
    vec4 offsets;
    vec3 scale;
    float opacity;
};

uniform Fade u_fade;
uniform sampler2D u_tex;

/* declared in the other order than the vertex shader writes them */
varying vec4 v_uv2;
varying float v_mix;

void main()
{
    vec4 a = texture2D(u_tex, v_uv2.xy);
    vec4 b = texture2D(u_tex, v_uv2.zw);
    gl_FragColor = mix(a, b, v_mix) * u_fade.opacity;
}
