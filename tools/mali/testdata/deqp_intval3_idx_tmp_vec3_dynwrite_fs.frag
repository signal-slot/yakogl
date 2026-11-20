varying mediump vec4 v_coords;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;

void main()
{
    mediump vec3 coords = vec3(v_coords);
    mediump vec3 arr[4];
    arr[ui_zero]  = vec3(coords);
    arr[ui_one]   = vec3(coords) * 0.5;
    arr[ui_two]   = vec3(coords) * 0.25;
    arr[ui_three] = vec3(coords) * 0.125;
    mediump vec3 res = vec3(0.0);
    res += arr[0];
    res += arr[1];
    res += arr[2];
    res += arr[3];
    gl_FragColor = vec4(res, 1.0);
}
