attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform int ui_one;

uniform mediump float s_a;
uniform mediump vec3 s_b;
uniform sampler2D s_c;

void main (void)
{
    v_color = vec4(texture2D(s_c, a_coords.xy * s_b.xy + s_b.z).rgb, s_a);
    gl_Position = a_position;
}
