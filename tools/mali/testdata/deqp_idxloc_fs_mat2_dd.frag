varying mediump vec4 v_coords;
uniform mediump int ui_zero, ui_one;
uniform mediump int ui_two;

void main()
{
    mediump vec4 coords = v_coords;
    mediump mat2 tmp;
    for (int i = 0; i < ui_two; i++)
    {
        tmp[i] = vec2(coords);
        coords = coords.yzwx * 0.5;
    }
    mediump vec2 res = vec2(0.0);
    res += tmp[ui_zero];
    res += tmp[ui_one];
    gl_FragColor = vec4(res, 0.0, 1.0);
}
