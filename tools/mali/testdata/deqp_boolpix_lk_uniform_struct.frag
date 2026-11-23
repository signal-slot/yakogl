precision mediump float;
struct Struct {mediump float a; mediump float b;};
uniform Struct val;
varying mediump float dummy;
bool isOk (float a, float b, float eps) { return (abs(a-b) <= (eps*abs(b) + eps)); }
uniform float ref_out0;
float out0;

void main()
{
    out0 = val.b + val.a;
    out0 = out0 + dummy;
    out0 = out0 - dummy;
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
