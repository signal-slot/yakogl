varying mediump vec4 v_coords;
uniform mediump vec4 u_arr[4];

void main()
{
    mediump vec4 res = vec4(0.0);
    res += u_arr[0];
    res += u_arr[1];
    res += u_arr[2];
    res += u_arr[3];
    gl_FragColor = vec4(res);
}
