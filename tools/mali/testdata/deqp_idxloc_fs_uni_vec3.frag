varying mediump vec4 v_coords;
uniform mediump int ui_four;
uniform mediump vec3 u_arr[4];

void main()
{
    mediump vec3 res = vec3(0.0);
    for (int i = 0; i < ui_four; i++)
        res += u_arr[i];
    gl_FragColor = vec4(res, 1.0);
}
