varying highp mat2 v_mat2;
uniform highp mat2 u_in1;

void main (void)
{
    highp mat2 res = v_mat2 + u_in1;
    gl_FragColor = vec4(res[0][0], res[1][0], res[0][1]+res[1][1], 1.0);
}
