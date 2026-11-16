attribute highp vec4 a_position;
attribute highp vec4 a_coords;
varying mediump vec4 v_color;
uniform mediump vec3 u_arr[4];

void main()
{
    gl_Position = a_position;
    mediump vec3 res = vec3(0.0);
    res += u_arr[0];
    res += u_arr[1];
    res += u_arr[2];
    res += u_arr[3];
    v_color = vec4(res, 1.0);
}
