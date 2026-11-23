precision mediump float;
precision mediump int;

bool isOk (bvec2 a, bvec2 b) { return (a == b); }
varying float v_in0;
varying float v_in1;
uniform bvec2 ref_out0;
bvec2 out0;


void main()
{
    bool in0 = bool(v_in0);
bool in1 = bool(v_in1);

    out0 = bvec2(in0, in1);
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
