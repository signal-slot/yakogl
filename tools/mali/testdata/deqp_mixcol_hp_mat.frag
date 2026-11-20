const highp mat2 in0 = mat2(0.6, 0.7, -1.0, 0.4);
varying highp float v_coords;

void main (void)
{
    highp mat2 res = in0 + v_coords;
    gl_FragColor = vec4(res[0][0], res[1][0], res[0][1]+res[1][1], 1.0);
}
