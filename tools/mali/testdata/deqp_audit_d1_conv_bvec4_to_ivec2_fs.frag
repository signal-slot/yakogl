precision mediump float;
precision mediump int;

bool isOk (ivec2 a, ivec2 b) { return (a == b); }
varying vec4 v_in0;
uniform ivec2 ref_out0;
ivec2 out0;


void main()
{
    bvec4 in0 = bvec4(v_in0);
    out0 = ivec2(in0);
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
