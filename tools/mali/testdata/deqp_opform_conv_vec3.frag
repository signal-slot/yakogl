precision mediump float;
precision mediump int;

bool isOk (vec3 a, vec3 b, float eps) { return all(lessThanEqual(abs(a-b), (eps*abs(b) + eps))); }
varying float v_in0;
varying float in1;
varying float v_in2;
uniform vec3 ref_out0;
vec3 out0;


void main()
{
    bool in0 = bool(v_in0);
int in2 = int(v_in2 * 1.0025);

    out0 = vec3(in0, in1, in2);
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
