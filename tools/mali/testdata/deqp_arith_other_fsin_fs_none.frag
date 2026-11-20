uniform mediump int u_numIters;
varying mediump vec4 v_coord;
void main (void)
{
    highp vec4 color = v_coord;
    for (int i = 0; i < 10; i++)
        color = sin(color);
    gl_FragColor = color;
}
