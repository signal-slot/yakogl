varying mediump vec4 v_coords;
uniform mediump vec2 u_arr[4];

void main()
{
    mediump vec2 res = vec2(0.0);
    res += u_arr[0];
    res += u_arr[1];
    res += u_arr[2];
    res += u_arr[3];
    gl_FragColor = vec4(res, 0.0, 1.0);
}
