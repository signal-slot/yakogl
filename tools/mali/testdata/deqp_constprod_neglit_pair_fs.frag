precision mediump float;
bool isOk (float a, float b, float eps) { return (abs(a-b) <= (eps*abs(b) + eps)); }
varying float in0;
uniform float ref_out0;
float out0;


float func (float a)
{
    return -a;
}

float func (float a, float b)
{
    return a * b;
}

void main()
{
    out0 = func(in0) * func(-0.5, -2.0);
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
