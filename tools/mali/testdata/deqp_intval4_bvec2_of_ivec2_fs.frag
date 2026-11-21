precision mediump float;
precision mediump int;

bool isOk (bvec2 a, bvec2 b) { return (a == b); }
varying vec4 v_in0;
uniform bvec2 ref_out0;
bvec2 out0;


void main()
{
    ivec2 in0 = ivec2(v_in0.xy * 1.0025);

    out0 = bvec2(in0);
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
