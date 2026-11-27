attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform int ui_one;

struct S {
    mediump float    a;
    mediump float    b[3];
    int                c;
};

void main (void)
{
    S s;
    s.a = a_coords.w;
    s.c = ui_one;
    s.b[0] = a_coords.z;
    s.b[1] = a_coords.y;
    s.b[2] = a_coords.x;
    v_color = vec4(s.a, s.b[0], s.b[1], s.c);
    gl_Position = a_position;
}
