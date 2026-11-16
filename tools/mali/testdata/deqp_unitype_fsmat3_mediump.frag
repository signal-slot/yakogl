uniform mediump mat3 u_in0;
varying mediump float v_coords;

void main (void)
{
    mediump mat3 res = u_in0 + v_coords;
    gl_FragColor = vec4(res[0]+res[1]+res[2], 1.0);
}
