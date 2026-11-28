varying mediump vec4 v_coords;
uniform int ui_zero;
uniform int ui_one;

struct T {
    int                a;
    mediump vec2    b;
};
struct S {
    mediump float    a;
    T                b;
    int                c;
};

S myFunc (void)
{
    S s = S(v_coords.x, T(0, vec2(0.0)), ui_one);
    s.b = T(ui_zero, v_coords.yz);
    return s;
}

void main (void)
{
    S s = myFunc();
    gl_FragColor = vec4(s.a, s.b.b, s.b.a + s.c);
    
}
