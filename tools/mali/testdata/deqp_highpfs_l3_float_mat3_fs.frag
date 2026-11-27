precision mediump float;

bool isOk (float a, float b, float eps) { return (abs(a-b) <= (eps*abs(b) + eps)); }
varying mat3 in0;
uniform float ref_out0;
float out0;

float func (mat3 a)
{
    return -(a[0][0] + a[0][1] + a[0][2] + a[1][0] + a[1][1] + a[1][2] + a[2][0] + a[2][1] + a[2][2]);
}

void main()
{
    out0 = func(in0);
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
