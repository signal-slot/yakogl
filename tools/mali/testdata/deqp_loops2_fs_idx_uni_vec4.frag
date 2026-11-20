varying mediump vec4 v_coords;
uniform mediump int ui_four;
uniform mediump vec4 u_arr[4];

void main()
{
    mediump vec4 res = vec4(0.0);
    for (int i = 0; i < ui_four; i++)
        res += u_arr[i];
    gl_FragColor = vec4(res);
}
