varying mediump mat4 v_mat4;
uniform mediump mat4 u_in1;

void main (void)
{
    mediump mat4 res = v_mat4 + u_in1;
    gl_FragColor = vec4(res[0].xyz+res[1].yzw+res[2].zwx+res[3].wxy, 1.0);
}
