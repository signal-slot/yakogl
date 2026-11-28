uniform highp mat3 u_in0;
uniform highp mat3 u_in1;

void main (void)
{
    highp mat3 res = u_in0 / u_in1;
    gl_FragColor = vec4(res[0]+res[1]+res[2], 1.0);
}
