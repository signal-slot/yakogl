varying mediump mat4 v_mat4;

void main (void)
{
    mediump mat4 tmpValue = v_mat4;
    mediump mat4 res = tmpValue--;
    gl_FragColor = vec4(res[0].xyz+res[1].yzw+res[2].zwx+res[3].wxy, 1.0) + vec4(tmpValue[0].xyz+tmpValue[1].yzw+tmpValue[2].zwx+tmpValue[3].wxy, 0.0);
}
