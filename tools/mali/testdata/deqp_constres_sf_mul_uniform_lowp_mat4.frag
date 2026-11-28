uniform lowp mat4 u_in0;
varying lowp float v_coords;

void main (void)
{
    lowp mat4 res = u_in0 * v_coords;
    gl_FragColor = vec4(res[0].xyz+res[1].yzw+res[2].zwx+res[3].wxy, 1.0);
}
