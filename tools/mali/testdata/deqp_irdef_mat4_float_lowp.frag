varying lowp mat4 v_mat4;
varying lowp float v_coords;

void main (void)
{
    lowp mat4 res = v_mat4 + v_coords;
    gl_FragColor = vec4(res[0].xyz+res[1].yzw+res[2].zwx+res[3].wxy, 1.0);
}
