varying mediump vec4 v_coords;
uniform int ui_one;

struct S {
    mediump float    a;
    mediump float    b[3];
    int                c;
};
uniform S s;

void main (void)
{
    gl_FragColor = vec4(s.a, s.b[0], s.b[1], s.c);
    
}
