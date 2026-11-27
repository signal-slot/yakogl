varying highp vec3 v_coords;
uniform highp mat3 u_in1;

void main (void)
{
    highp vec3 res = v_coords * u_in1;
    gl_FragColor = vec4(res, 1.0);
}
