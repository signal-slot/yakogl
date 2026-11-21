attribute highp vec4 dEQP_Position;
uniform bool u_false;

varying mediump float var;
void main()
{
    if (u_false)
        var = 1.0;
    gl_Position = dEQP_Position;

}
