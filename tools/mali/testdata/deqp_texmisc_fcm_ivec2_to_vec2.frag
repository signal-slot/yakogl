precision mediump float;
precision mediump int;

bool isOk (vec2 a, vec2 b, float eps) { return all(lessThanEqual(abs(a-b), (eps*abs(b) + eps))); }
varying vec2 v_in0;
uniform vec2 ref_out0;
vec2 out0;


void main()
{
    ivec2 in0 = ivec2(v_in0 * 1.0025);

    out0 = vec2(in0);
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
