varying mediump vec4 v_coords;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;

void main()
{
    mediump float coords = float(v_coords);
    mediump float arr[4];
    arr[ui_zero]  = float(coords);
    arr[ui_one]   = float(coords) * 0.5;
    arr[ui_two]   = float(coords) * 0.25;
    arr[ui_three] = float(coords) * 0.125;
    mediump float res = float(0.0);
    res += arr[ui_zero];
    res += arr[ui_one];
    res += arr[ui_two];
    res += arr[ui_three];
    gl_FragColor = vec4(res, 0.0, 0.0, 1.0);
}
