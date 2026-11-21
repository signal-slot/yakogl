varying mediump vec4 v_coords;
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
    S s = S(v_coords.x, v_coords.yzw, ui_zero);
    if (uf_one > 0.0)
        s = S(v_coords.w, v_coords.zyx, ui_one);
    gl_FragColor = vec4(s.a, s.b.xy, s.c);
    
}
