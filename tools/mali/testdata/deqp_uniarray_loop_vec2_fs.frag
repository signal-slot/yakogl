varying mediump vec4 v_coords;
uniform mediump vec2 u_arr[4];

void main()
{
    mediump vec2 res = vec2(0.0);
    for (int i = 0; i < 4; i++)
        res += u_arr[i];
    gl_FragColor = vec4(res, 0.0, 1.0);
}
