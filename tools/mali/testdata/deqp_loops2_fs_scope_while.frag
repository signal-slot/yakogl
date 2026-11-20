#version 100
precision mediump float;
bool isOk (int a, int b)     { return (a == b); }
varying float v_in0;
uniform int ref_out0;
int out0;

void main()
{
    int in0 = int(v_in0 * 1.0025);

    int a = in0;
    int i = 0;
    while (bool a = (i < 1))
    {
        i++;
    }
    out0 = a;
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
