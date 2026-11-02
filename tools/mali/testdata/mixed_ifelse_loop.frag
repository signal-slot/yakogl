#version 100
precision highp float;
uniform vec4 u_a;
uniform vec4 u_b;
uniform int u_n;
void main()
{
    vec4 c = vec4(0.0);
    for (int i = 0; i < 8; i++)
    {
        if (i >= u_n)
            break;
        if (u_a.x > float(i))
            c += u_a * u_b;
        else
            c -= u_b.yzwx;
    }
    gl_FragColor = c;
}
