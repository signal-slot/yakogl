varying mediump vec4 v_coords;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;

void main()
{
    mediump vec2 coords = vec2(v_coords);
    mediump vec2 arr[4];
    arr[ui_zero]  = vec2(coords);
    arr[ui_one]   = vec2(coords) * 0.5;
    arr[ui_two]   = vec2(coords) * 0.25;
    arr[ui_three] = vec2(coords) * 0.125;
    mediump vec2 res = vec2(0.0);
    for (int i = 0; i < 4; i++)
        res += arr[i];
    gl_FragColor = vec4(res, 0.0, 1.0);
}
