precision mediump float;
struct Struct {mediump vec4 a; mediump vec4 b;};
uniform Struct val;
bool isOk (float a, float b, float eps) { return (abs(a-b) <= (eps*abs(b) + eps)); }
uniform float ref_out0;
float out0;

varying mediump float res;
void main()
{
    out0 = res + val.b.y;
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
