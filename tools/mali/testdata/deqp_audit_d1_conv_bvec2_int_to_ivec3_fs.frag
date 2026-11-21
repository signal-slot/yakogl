precision mediump float;
precision mediump int;

bool isOk (ivec3 a, ivec3 b) { return (a == b); }
varying vec2 v_in0;
varying float v_in1;
uniform ivec3 ref_out0;
ivec3 out0;


void main()
{
    bvec2 in0 = bvec2(v_in0);
    int in1 = int(v_in1 * 1.0025);
    out0 = ivec3(in0, in1);
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
