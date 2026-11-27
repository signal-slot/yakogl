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

void main (void)
{
    S s[3];
    s[0] = S(a_coords.x, ui_zero);
    s[1].a = a_coords.y;
    s[1].b = ui_one;
    s[2] = S(a_coords.z, ui_two);
    v_color = vec4(s[ui_two].a, s[ui_one].a, s[ui_zero].a, s[ui_two].b - s[ui_one].b + s[ui_zero].b);
    gl_Position = a_position;
}
