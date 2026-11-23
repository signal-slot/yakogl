precision mediump float;

bool isOk (bvec3 a, bvec3 b) { return (a == b); }
varying vec3 v_in0;
uniform bvec3 ref_out0;
bvec3 out0;


void main()
{
    bvec3 in0 = bvec3(v_in0);

    out0 = in0.zyx;
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
