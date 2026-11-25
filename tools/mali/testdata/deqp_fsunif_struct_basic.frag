varying mediump vec4 v_coords;
uniform int ui_one;

struct S {
    mediump float    a;
    mediump vec3    b;
    int                c;
};
uniform S s;

void main (void)
{
    gl_FragColor = vec4(s.a, s.b.x, s.b.y, s.c);
    
}
