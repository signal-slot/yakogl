precision mediump float;
precision mediump int;

bool isOk (vec3 a, vec3 b, float eps) { return all(lessThanEqual(abs(a-b), (eps*abs(b) + eps))); }
varying vec3 v_in0;
uniform vec3 ref_out0;
vec3 out0;


void main()
{
    ivec3 in0 = ivec3(v_in0 * 1.0025);

    out0 = vec3(in0);
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
