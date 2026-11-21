precision mediump float;

bool isOk (vec2 a, vec2 b, float eps) { return all(lessThanEqual(abs(a-b), (eps*abs(b) + eps))); }
varying vec3 in0;
uniform vec2 ref_out0;
vec2 out0;


void main()
{

    out0 = in0.sp;
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
