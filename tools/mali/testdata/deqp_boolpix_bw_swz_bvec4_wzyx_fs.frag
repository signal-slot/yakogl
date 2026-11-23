precision mediump float;

bool isOk (bvec4 a, bvec4 b) { return (a == b); }
varying vec4 v_in0;
uniform bvec4 ref_out0;
bvec4 out0;


void main()
{
    bvec4 in0 = bvec4(v_in0);

    out0 = in0.wzyx;
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
