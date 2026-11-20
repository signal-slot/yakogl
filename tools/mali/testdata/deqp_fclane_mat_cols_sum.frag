varying highp mat3 v_mat3;
varying highp float v_coords;

void main (void)
{
    highp mat3 res = v_mat3 + v_coords;
    gl_FragColor = vec4(res[0]+res[1]+res[2], 1.0);
}
