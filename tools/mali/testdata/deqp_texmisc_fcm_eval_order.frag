precision mediump float;
bool isOk (int a, int b)     { return (a == b); }
varying float v_in0;
uniform int ref_out0;
int out0;


int func (float a, int b, bool c, int d)
{
    if (c)
        return b + int(a) + d;
    else
        return -1;
}

void main ()
{
    int in0 = int(v_in0 * 1.0025);

    float v0 = float(in0);
    int v1 = in0;
    out0 = func((v0 += 1.0), v1++, (v0 > 1.5), v1);
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
