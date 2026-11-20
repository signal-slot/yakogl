varying mediump vec4 v_coords;
uniform int ui_one;

struct S {
    mediump float    a;
    mediump vec3    b;
    int                c;
};

void main (void)
{
    S s = S(v_coords.x, vec3(0.0), ui_one);
    s.b = v_coords.yzw;
    gl_FragColor = vec4(s.a, s.b.x, s.b.y, s.c);
    
}
