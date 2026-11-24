attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform int ui_one;

struct S {
    mediump float    a;
    mediump vec3    b;
    int                c;
};

void main (void)
{
    S s = S(a_coords.x, vec3(0.0), ui_one);
    s.b = a_coords.yzw;
    v_color = vec4(s.a, s.b.x, s.b.y, s.c);
    gl_Position = a_position;
}
