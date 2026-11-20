varying mediump vec4 v_coords;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;
uniform mediump int ui_four;

void main()
{
    mediump vec4 coords = v_coords;
    mediump mat4 tmp;
    for (int i = 0; i < ui_four; i++)
    {
        tmp[i] = vec4(coords);
        coords = coords.yzwx * 0.5;
    }
    mediump vec4 res = vec4(0.0);
    res += tmp[ui_zero];
    res += tmp[ui_one];
    res += tmp[ui_two];
    res += tmp[ui_three];
    gl_FragColor = vec4(res);
}
