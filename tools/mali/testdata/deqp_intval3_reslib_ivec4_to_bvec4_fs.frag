precision mediump float;
precision mediump int;

bool isOk (bvec4 a, bvec4 b) { return (a == b); }
varying vec4 v_in0;
uniform bvec4 ref_out0;
bvec4 out0;


void main()
{
    ivec4 in0 = ivec4(v_in0 * 1.0025);

    out0 = bvec4(in0);
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
