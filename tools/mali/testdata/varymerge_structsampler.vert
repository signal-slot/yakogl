/* dEQP shaders.struct.uniform.sampler_vertex (es2fShaderStructTests.cpp,
 * Apache-2.0): a texture look-up in the vertex shader, through a sampler
 * in a uniform struct. es100/va.frag draws v_color. */
attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform int ui_one;

struct S {
    mediump float    a;
    mediump vec3    b;
    sampler2D        c;
};
uniform S s;

void main (void)
{
    v_color = vec4(texture2D(s.c, a_coords.xy * s.b.xy + s.b.z).rgb, s.a);
    gl_Position = a_position;
}
