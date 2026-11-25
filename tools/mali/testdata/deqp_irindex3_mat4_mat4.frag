varying mediump mat4 v_m0;
varying mediump mat4 v_m1;

void main (void)
{
    mediump mat4 res = v_m0 + v_m1;
    gl_FragColor = vec4(res[0].xyz+res[1].yzw+res[2].zwx+res[3].wxy, 1.0);
}
