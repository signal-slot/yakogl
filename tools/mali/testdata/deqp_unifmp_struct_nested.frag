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
uniform S s;

void main (void)
{
    gl_FragColor = vec4(s.a, s.b.b, s.b.a + s.c);
    
}
