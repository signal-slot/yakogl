attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform int ui_one;

uniform mediump float s0_a;
uniform mediump vec3 s0_b;
uniform sampler2D s0_c;
uniform mediump float s1_a;
uniform mediump vec3 s1_b;
uniform sampler2D s1_c;

void main (void)
{
    v_color = vec4(texture2D(s1_c, a_coords.xy * s0_b.xy + s1_b.z).rgb, s0_a);
    gl_Position = a_position;
}
