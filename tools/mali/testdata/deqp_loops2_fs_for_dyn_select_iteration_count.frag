varying mediump vec4 v_coords;
varying mediump float v_one;
uniform bool ub_true;
uniform mediump int ui_zero, ui_one, ui_two, ui_three, ui_four, ui_five, ui_six;

void main()
{
    mediump int one = int(v_one + 0.5);
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
    for (int i = 0; i < (ub_true ? one*ui_three : 0); i++) res = res.yzwx;
    gl_FragColor = vec4(res.rgb, 1.0);
}
