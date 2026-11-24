precision mediump float;
precision mediump int;

bool isOk (bvec3 a, bvec3 b) { return (a == b); }
varying float v_in0;
varying float in1;
varying float v_in2;
uniform bvec3 ref_out0;
bvec3 out0;


void main()
{
    bool in0 = bool(v_in0);
int in2 = int(v_in2 * 1.0025);

    out0 = bvec3(in0, in1, in2);
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
