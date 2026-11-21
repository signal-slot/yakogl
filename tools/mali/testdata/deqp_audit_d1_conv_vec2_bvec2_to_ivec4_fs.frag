precision mediump float;
precision mediump int;

bool isOk (ivec4 a, ivec4 b) { return (a == b); }
varying vec2 in0;
varying vec2 v_in1;
uniform ivec4 ref_out0;
ivec4 out0;


void main()
{
    bvec2 in1 = bvec2(v_in1);
    out0 = ivec4(in0, in1);
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
