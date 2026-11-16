const mediump int line = __LINE__;
precision mediump float;
bool isOk (float a, float b, float eps) { return (abs(a-b) <= (eps*abs(b) + eps)); }
uniform float ref_out0;
float out0;

void main()
{
    out0 = float(line);
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
