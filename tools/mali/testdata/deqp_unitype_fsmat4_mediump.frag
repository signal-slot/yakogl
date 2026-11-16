uniform mediump mat4 u_in0;
varying mediump vec4 v_coords;

void main (void)
{
    mediump vec4 res = u_in0 * v_coords;
    gl_FragColor = vec4(res.x, res.y, res.z+res.w, 1.0);
}
