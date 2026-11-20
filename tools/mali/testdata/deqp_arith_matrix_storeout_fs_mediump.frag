const mediump mat3 in0 = mat3(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.2, -0.3, 1.0);
varying mediump float v_coords;

void main (void)
{
    mediump mat3 res = in0 + v_coords;
    gl_FragColor = vec4(res[0]+res[1]+res[2], 1.0);
}
