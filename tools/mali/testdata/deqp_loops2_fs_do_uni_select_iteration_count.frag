varying mediump vec4 v_coords;
uniform bool ub_true;
uniform mediump int ui_zero, ui_one, ui_two, ui_three, ui_four, ui_five, ui_six;

void main()
{
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
    mediump int i = 0;
    do { res = res.yzwx; } while (++i < (ub_true ? ui_three : 0));
    gl_FragColor = vec4(res.rgb, 1.0);
}
