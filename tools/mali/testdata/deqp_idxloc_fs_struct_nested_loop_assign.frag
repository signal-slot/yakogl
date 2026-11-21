varying mediump vec4 v_coords;
uniform int ui_zero;
uniform int ui_one;
uniform int ui_three;
uniform mediump float uf_one;

struct T {
    int                a;
    mediump vec2    b;
};
struct S {
    mediump float    a;
    T                b;
    int                c;
};

void main (void)
{
    S s = S(v_coords.x, T(ui_one, v_coords.yz), ui_one);
    for (int i = 0; i < ui_three; i++)
    {
        if (i == ui_one)
            s.b = T(ui_zero, v_coords.zw);
    }
    gl_FragColor = vec4(s.a, s.b.b, s.c - s.b.a);
    
}
