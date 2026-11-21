varying mediump vec4 v_coords;
uniform mediump int ui_four;

void main()
{
    mediump vec2 coords = vec2(v_coords);
    mediump vec2 arr[4];
    for (int i = 0; i < ui_four; i++)
    {
        arr[i] = vec2(coords);
        coords = coords * 0.5;
    }
    mediump vec2 res = vec2(0.0);
    for (int i = 0; i < 4; i++)
        res += arr[i];
    gl_FragColor = vec4(res, 0.0, 1.0);
}
