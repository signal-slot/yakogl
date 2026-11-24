attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform int ui_zero;
uniform int ui_one;

struct T {
    int                a;
    mediump vec2    b;
};
struct S {
    mediump float    a;
    T                b;
    int                c;
};
uniform S s;

void main (void)
{
    v_color = vec4(s.a, s.b.b, s.b.a + s.c);
    gl_Position = a_position;
}
