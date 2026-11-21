precision mediump float;
struct Struct {mediump vec4 a; mediump vec4 b;};
uniform Struct val;
varying mediump float dummy;
uniform float ref_out0;
float out0;
void main()
{
    out0 = val.b.y + val.a.x;
    out0 = out0 + dummy;
    out0 = out0 - dummy;
    bool RES = abs(out0 - ref_out0) <= (0.05 * abs(ref_out0) + 0.05);
    gl_FragColor = vec4(RES, RES, RES, 1.0);
}
