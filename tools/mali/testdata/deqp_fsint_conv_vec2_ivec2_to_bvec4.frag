precision mediump float;
precision mediump int;

bool isOk (bvec4 a, bvec4 b) { return (a == b); }
varying vec2 in0;
varying vec2 v_in1;
uniform bvec4 ref_out0;
bvec4 out0;


void main()
{
    ivec2 in1 = ivec2(v_in1 * 1.0025);

    out0 = bvec4(in0, in1);
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
