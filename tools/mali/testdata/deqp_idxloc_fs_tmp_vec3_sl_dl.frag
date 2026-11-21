varying mediump vec4 v_coords;
uniform mediump int ui_four;

void main()
{
    mediump vec3 coords = vec3(v_coords);
    mediump vec3 arr[4];
    for (int i = 0; i < 4; i++)
    {
        arr[i] = vec3(coords);
        coords = coords * 0.5;
    }
    mediump vec3 res = vec3(0.0);
    for (int i = 0; i < ui_four; i++)
        res += arr[i];
    gl_FragColor = vec4(res, 1.0);
}
