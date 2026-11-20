varying mediump vec4 v_coords;
uniform int ui_zero;
uniform int ui_one;
uniform int ui_two;
uniform int ui_three;

struct S {
    mediump float    a;
    mediump int        b;
};

void main (void)
{
    S s[3];
    s[0] = S(v_coords.x, ui_zero);
    s[1].a = v_coords.y;
    s[1].b = -ui_one;
    s[2] = S(v_coords.z, ui_two);

    mediump float rgb[3];
    int alpha = 0;
    for (int i = 0; i < ui_three; i++)
    {
        rgb[i] = s[2-i].a;
        alpha += s[i].b;
    }
    gl_FragColor = vec4(rgb[0], rgb[1], rgb[2], alpha);
    
}
