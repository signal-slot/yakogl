precision highp float;
varying vec4 v;
void main()
{
    vec4 c = v;
    for (int i = 0; i < 40; i++)
        c = c * v;
    gl_FragColor = c;
}
