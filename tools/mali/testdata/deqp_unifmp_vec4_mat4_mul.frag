varying mediump vec4 v_coords;
uniform mediump mat4 u_in1;

void main (void)
{
    mediump vec4 res = v_coords * u_in1;
    gl_FragColor = vec4(res.x, res.y, res.z+res.w, 1.0);
}
