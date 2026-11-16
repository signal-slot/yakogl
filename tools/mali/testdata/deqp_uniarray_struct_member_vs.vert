attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform int ui_one;

struct S {
    mediump float    a;
    mediump float    b[3];
    int                c;
};
uniform S s;

void main (void)
{
    v_color = vec4(s.a, s.b[0], s.b[1], s.c);
    gl_Position = a_position;
}
