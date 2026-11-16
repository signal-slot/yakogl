varying mediump vec4 v_coords;
uniform mediump vec3 u_arr[4];

void main()
{
    mediump vec3 res = vec3(0.0);
    res += u_arr[0];
    res += u_arr[1];
    res += u_arr[2];
    res += u_arr[3];
    gl_FragColor = vec4(res, 1.0);
}
