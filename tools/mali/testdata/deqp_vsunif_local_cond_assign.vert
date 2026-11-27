attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform int ui_zero;
uniform int ui_one;
uniform mediump float uf_one;

struct S {
    mediump float    a;
    mediump vec3    b;
    int                c;
};

void main (void)
{
    S s = S(a_coords.x, a_coords.yzw, ui_zero);
    if (uf_one > 0.0)
        s = S(a_coords.w, a_coords.zyx, ui_one);
    v_color = vec4(s.a, s.b.xy, s.c);
    gl_Position = a_position;
}
