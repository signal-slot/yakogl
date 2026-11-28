varying mediump vec4 v_coords;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;
uniform mediump int ui_four;

void main()
{
    mediump vec4 coords = vec4(v_coords);
    mediump vec4 arr[4];
    arr[ui_zero]  = vec4(coords);
    arr[ui_one]   = vec4(coords) * 0.5;
    arr[ui_two]   = vec4(coords) * 0.25;
    arr[ui_three] = vec4(coords) * 0.125;
    mediump vec4 res = vec4(0.0);
    for (int i = 0; i < ui_four; i++)
        res += arr[i];
    gl_FragColor = vec4(res);
}
