attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
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
    mediump float r = (s[0].b[1].b[0].x + s[1].b[2].b[1].y) * s[0].b[0].a; // (z + z) * 0.5
    mediump float g = s[1].b[0].b[0].y * s[0].b[2].a * s[1].b[2].a; // x * 0.25 * 4
    mediump float b = (s[0].b[2].b[1].y + s[0].b[1].b[0].y + s[1].a) * s[0].b[1].a; // (w + w + w) * 0.333
    mediump float a = float(s[0].c) + s[1].b[2].a - s[1].b[1].a; // 0 + 4.0 - 3.0
    v_color = vec4(r, g, b, a);
    gl_Position = a_position;
}
