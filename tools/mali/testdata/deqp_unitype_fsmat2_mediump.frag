uniform mediump mat2 u_in0;
uniform mediump mat2 u_in1;

void main (void)
{
    mediump mat2 res = u_in0 + u_in1;
    gl_FragColor = vec4(res[0][0], res[1][0], res[0][1]+res[1][1], 1.0);
}
