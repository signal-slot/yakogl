varying mediump vec4 v_coords;
uniform mediump int ui_zero, ui_one, ui_two, ui_three, ui_four, ui_five, ui_six;

void main()
{
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
    for (mediump int i = 0; i < ui_five; i++) { if (i == 0) continue; else if (i == 3) break; res = res.yzwx; }
    gl_FragColor = vec4(res.rgb, 1.0);
}
