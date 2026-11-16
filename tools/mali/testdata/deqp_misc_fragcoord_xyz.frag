uniform mediump vec3 u_scale;
void main (void)
{
    gl_FragColor = vec4(gl_FragCoord.xyz*u_scale, 1.0);
}
