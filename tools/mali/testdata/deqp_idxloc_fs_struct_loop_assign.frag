varying mediump vec4 v_coords;
uniform int ui_zero;
uniform int ui_one;
uniform int ui_three;

struct S {
    mediump float    a;
    mediump vec3    b;
    int                c;
};

void main (void)
{
    S s = S(v_coords.x, v_coords.yzw, ui_zero);
    for (int i = 0; i < ui_three; i++)
    {
        if (i == ui_one)
            s = S(v_coords.w, v_coords.zyx, ui_one);
    }
    gl_FragColor = vec4(s.a, s.b.xy, s.c);
    
}
