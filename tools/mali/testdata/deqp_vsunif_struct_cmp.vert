attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform mediump float uf_one;
uniform int ui_two;

struct S {
    mediump float    a;
    mediump vec3    b;
    int                c;
};
uniform S a;
uniform S b;
uniform S c;

void main (void)
{
    S d = S(uf_one, vec3(0.0, floor(a_coords.y+1.0), 2.0), ui_two);
    v_color = vec4(0.0, 0.0, 0.0, 1.0);
    if (a == b) v_color.x = 1.0;
    if (a == c) v_color.y = 1.0;
    if (a == d) v_color.z = 1.0;
    gl_Position = a_position;
}
