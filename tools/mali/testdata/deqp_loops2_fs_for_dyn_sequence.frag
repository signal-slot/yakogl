varying mediump vec4 v_coords;
varying mediump float v_one;
uniform mediump int ui_zero, ui_one, ui_two, ui_three, ui_four, ui_five, ui_six;

void main()
{
    mediump int one = int(v_one + 0.5);
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
    mediump int i;
    for (i = 0; i < one*ui_two; i++) { res = res.yzwx; }
    for (; i < one*ui_five; i++) { res = res.yzwx; }
    gl_FragColor = vec4(res.rgb, 1.0);
}
