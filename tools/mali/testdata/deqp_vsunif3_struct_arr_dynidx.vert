attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform int ui_zero;
uniform int ui_one;
uniform int ui_two;

struct T {
    mediump float    a;
    mediump vec2    b[2];
};
struct S {
    mediump float    a;
    T                b[3];
    int                c;
};
uniform S s[2];

void main (void)
{
    mediump float r = (s[0].b[ui_one].b[ui_one-1].x + s[ui_one].b[ui_two].b[ui_zero+1].y) * s[0].b[0].a; // (z + z) * 0.5
    mediump float g = s[ui_two-1].b[ui_two-2].b[ui_zero].y * s[0].b[ui_two].a * s[ui_one].b[2].a; // x * 0.25 * 4
    mediump float b = (s[ui_zero].b[ui_one+1].b[1].y + s[0].b[ui_one*ui_one].b[0].y + s[ui_one].a) * s[0].b[ui_two-ui_one].a; // (w + w + w) * 0.333
    mediump float a = float(s[ui_zero].c) + s[ui_one-ui_zero].b[ui_two].a - s[ui_zero+ui_one].b[ui_two-ui_one].a; // 0 + 4.0 - 3.0
    v_color = vec4(r, g, b, a);
    gl_Position = a_position;
}
