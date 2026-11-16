const highp mat4 in0 = mat4(1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.2, -0.3, 0.2, 1.0);
varying highp float v_coords;

void main (void)
{
    highp mat4 res = in0 * v_coords;
    gl_FragColor = vec4(res[0].xyz+res[1].yzw+res[2].zwx+res[3].wxy, 1.0);
}
