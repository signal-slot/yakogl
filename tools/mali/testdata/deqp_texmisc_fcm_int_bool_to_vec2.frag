precision mediump float;
precision mediump int;

bool isOk (vec2 a, vec2 b, float eps) { return all(lessThanEqual(abs(a-b), (eps*abs(b) + eps))); }
varying float v_in0;
varying float v_in1;
uniform vec2 ref_out0;
vec2 out0;


void main()
{
    int in0 = int(v_in0 * 1.0025);
bool in1 = bool(v_in1);

    out0 = vec2(in0, in1);
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
