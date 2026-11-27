precision mediump float;
precision mediump int;
bool isOk (float a, float b, float eps) { return (abs(a-b) <= (eps*abs(b) + eps)); }
varying float in0;
uniform float ref_out0;
float out0;


void func (out highp float a)
{
    a = -1.0;
}

void main()
{
    
    float f = 1.0;
    func(f);
    out0 = f * in0;
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
