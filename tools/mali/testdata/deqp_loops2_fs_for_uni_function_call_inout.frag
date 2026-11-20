varying mediump vec4 v_coords;
uniform mediump int ui_zero, ui_one, ui_two, ui_three, ui_four, ui_five, ui_six;

void func (inout mediump vec4 coords) { coords = coords.yzwx; }

void main()
{
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
    for (mediump int i = 0; i < ui_three; i++)
    {
        func(res);
    }
    gl_FragColor = vec4(res.rgb, 1.0);
}
