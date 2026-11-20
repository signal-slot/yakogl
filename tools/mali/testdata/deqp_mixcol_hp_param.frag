precision mediump float;
precision mediump int;
bool isOk (float a, float b, float eps) { return (abs(a-b) <= (eps*abs(b) + eps)); }
varying float in0;
uniform float ref_out0;
float out0;


float func (in highp float a)
{
    a = -a;
    return 2.0 * a;
}

void main()
{
    
    float f = in0;
    float g = func(f);
    out0 = f + g;
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
