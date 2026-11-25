varying mediump mat4 v_mat4;

void main (void)
{
    mediump mat4 m = v_mat4;
    mediump mat4 res = ++m;
    gl_FragColor = vec4(res[0].xyz+res[1].yzw+res[2].zwx+res[3].wxy, 1.0);
}
