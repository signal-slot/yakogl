varying mediump mat3 v_mat3;
uniform mediump float u_in1;

void main (void)
{
    mediump mat3 res = v_mat3 / u_in1;
    gl_FragColor = vec4(res[0]+res[1]+res[2], 1.0);
}
