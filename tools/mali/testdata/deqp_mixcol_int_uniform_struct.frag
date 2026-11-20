varying mediump vec4 v_coords;
uniform int ui_zero;
uniform int ui_one;
uniform int ui_two;
uniform mediump float uf_two;
uniform mediump float uf_three;
uniform mediump float uf_four;
uniform mediump float uf_half;
uniform mediump float uf_third;
uniform mediump float uf_fourth;

struct T {
    mediump float    a;
    mediump vec2    b[2];
};
struct S {
    mediump float    a;
    T                b[3];
    int                c;
};

void main (void)
{
    S s[2];

    // S[0]
    s[0].a         = v_coords.x;
    s[0].b[0].a    = uf_half;
    s[0].b[0].b[0] = v_coords.xy;
    s[0].b[0].b[1] = v_coords.zw;
    s[0].b[1].a    = uf_third;
    s[0].b[1].b[0] = v_coords.zw;
    s[0].b[1].b[1] = v_coords.xy;
    s[0].b[2].a    = uf_fourth;
    s[0].b[2].b[0] = v_coords.xz;
    s[0].b[2].b[1] = v_coords.yw;
    s[0].c         = ui_zero;

    // S[1]
    s[1].a         = v_coords.w;
    s[1].b[0].a    = uf_two;
    s[1].b[0].b[0] = v_coords.xx;
    s[1].b[0].b[1] = v_coords.yy;
    s[1].b[1].a    = uf_three;
    s[1].b[1].b[0] = v_coords.zz;
    s[1].b[1].b[1] = v_coords.ww;
    s[1].b[2].a    = uf_four;
    s[1].b[2].b[0] = v_coords.yx;
    s[1].b[2].b[1] = v_coords.wz;
    s[1].c         = ui_one;

    mediump float r = (s[0].b[1].b[0].x + s[1].b[2].b[1].y) * s[0].b[0].a; // (z + z) * 0.5
    mediump float g = s[1].b[0].b[0].y * s[0].b[2].a * s[1].b[2].a; // x * 0.25 * 4
    mediump float b = (s[0].b[2].b[1].y + s[0].b[1].b[0].y + s[1].a) * s[0].b[1].a; // (w + w + w) * 0.333
    mediump float a = float(s[0].c) + s[1].b[2].a - s[1].b[1].a; // 0 + 4.0 - 3.0
    gl_FragColor = vec4(r, g, b, a);
    
}
