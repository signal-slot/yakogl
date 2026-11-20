varying mediump float var;
uniform bool u_false;
void main()
{
    if (u_false)
        gl_FragColor = vec4(var);
    else
        gl_FragColor = vec4(1.0);
}
