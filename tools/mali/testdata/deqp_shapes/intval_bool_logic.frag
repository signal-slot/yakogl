precision mediump float;
bool isOk (float a, float b, float eps) { return (abs(a-b) <= (eps*abs(b) + eps)); }
varying float in0;
uniform float ref_out0;
float out0;
uniform float ref_out1;
float out1;

void main()
{
    out0 = 0.0;
    out1 = 1.0;
    if (in0 >= 1.0)
    {
        out0 = 1.0;
        out1 = 0.0;
    }
    bool RES = isOk(out0, ref_out0, 0.05);
RES = RES && isOk(out1, ref_out1, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
