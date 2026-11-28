precision mediump float;
precision mediump int;

bool isOk (vec4 a, vec4 b, float eps) { return all(lessThanEqual(abs(a-b), (eps*abs(b) + eps))); }
varying vec4 v_in0;
uniform vec4 ref_out0;
vec4 out0;


void main()
{
    ivec4 in0 = ivec4(v_in0 * 1.0025);

    out0 = vec4(in0);
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
