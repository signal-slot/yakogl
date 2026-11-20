varying highp mat2 v_mat2;
varying highp float v_coords;

void main (void)
{
    highp mat2 res = v_mat2 + v_coords;
    gl_FragColor = vec4(res[0][0], res[1][0], res[0][1]+res[1][1], 1.0);
}
