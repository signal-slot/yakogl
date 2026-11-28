varying mediump vec4 v_coords;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;

void main()
{
    mediump vec4 coords = vec4(v_coords);
    mediump vec4 arr[4];
    arr[ui_zero]  = vec4(coords);
    arr[ui_one]   = vec4(coords) * 0.5;
    arr[ui_two]   = vec4(coords) * 0.25;
    arr[ui_three] = vec4(coords) * 0.125;
    mediump vec4 res = vec4(0.0);
    res += arr[ui_zero];
    res += arr[ui_one];
    res += arr[ui_two];
    res += arr[ui_three];
    gl_FragColor = vec4(res);
}
