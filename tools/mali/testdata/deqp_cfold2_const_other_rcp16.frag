const mediump mat2 in0 = mat2(0.6, 0.7, -1.0, 0.4);
const mediump mat2 in1 = mat2(-0.5, 0.7, -0.4, -0.8);

void main (void)
{
    mediump mat2 res = in0 / in1;
    gl_FragColor = vec4(res[0][0], res[1][0], res[0][1]+res[1][1], 1.0);
}
