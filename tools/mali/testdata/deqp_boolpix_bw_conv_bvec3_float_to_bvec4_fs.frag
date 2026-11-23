precision mediump float;
precision mediump int;

bool isOk (bvec4 a, bvec4 b) { return (a == b); }
varying vec3 v_in0;
varying float in1;
uniform bvec4 ref_out0;
bvec4 out0;


void main()
{
    bvec3 in0 = bvec3(v_in0);

    out0 = bvec4(in0, in1);
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
