varying mediump vec4 v_coords;
uniform int ui_zero;
uniform int ui_one;

struct T {
    sampler2D        a;
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
    gl_FragColor = vec4(texture2D(s.b.a, v_coords.xy * s.b.b + s.a).rgb, s.c);
    
}
