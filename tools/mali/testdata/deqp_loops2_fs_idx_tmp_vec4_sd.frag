varying mediump vec4 v_coords;
uniform mediump int ui_four;

void main()
{
    mediump vec4 coords = vec4(v_coords);
    mediump vec4 arr[4];
    arr[0] = vec4(coords);
    arr[1] = vec4(coords) * 0.5;
    arr[2] = vec4(coords) * 0.25;
    arr[3] = vec4(coords) * 0.125;
    mediump vec4 res = vec4(0.0);
    for (int i = 0; i < ui_four; i++)
        res += arr[i];
    gl_FragColor = vec4(res);
}
