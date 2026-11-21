varying mediump vec4 v_coords;
uniform int ui_zero;
uniform int ui_one;
uniform int ui_two;
uniform int ui_three;
uniform mediump float uf_two;
uniform mediump float uf_three;
uniform mediump float uf_four;
uniform mediump float uf_half;
uniform mediump float uf_third;
uniform mediump float uf_fourth;
uniform mediump float uf_sixth;

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
    mediump float r = 0.0; // (x*3 + y*3) / 6.0
    mediump float g = 0.0; // (y*3 + z*3) / 6.0
    mediump float b = 0.0; // (z*3 + w*3) / 6.0
    mediump float a = 1.0;
    for (int i = 0; i < ui_two; i++)
    {
        for (int j = 0; j < ui_three; j++)
        {
            r += s[0].b[j].b[i].y;
            g += s[i].b[j].b[0].x;
            b += s[i].b[j].b[1].x;
            a *= s[i].b[j].a;
        }
    }
    gl_FragColor = vec4(r*uf_sixth, g*uf_sixth, b*uf_sixth, a);
    
}
