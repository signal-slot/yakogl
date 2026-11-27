attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform int ui_zero;
uniform int ui_one;
uniform int ui_two;

struct S {
    mediump float    a;
    mediump int        b;
};
uniform S s[3];

void main (void)
{
    v_color = vec4(s[2].a, s[1].a, s[0].a, s[2].b - s[1].b + s[0].b);
    gl_Position = a_position;
}
