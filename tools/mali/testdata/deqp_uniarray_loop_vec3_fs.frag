varying mediump vec4 v_coords;
uniform mediump vec3 u_arr[4];

void main()
{
    mediump vec3 res = vec3(0.0);
    for (int i = 0; i < 4; i++)
        res += u_arr[i];
    gl_FragColor = vec4(res, 1.0);
}
