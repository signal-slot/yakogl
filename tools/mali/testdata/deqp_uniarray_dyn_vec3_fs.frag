varying mediump vec4 v_coords;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;
uniform mediump vec3 u_arr[4];

void main()
{
    mediump vec3 res = vec3(0.0);
    res += u_arr[ui_zero];
    res += u_arr[ui_one];
    res += u_arr[ui_two];
    res += u_arr[ui_three];
    gl_FragColor = vec4(res, 1.0);
}
