precision mediump float;
precision mediump int;

bool isOk (ivec3 a, ivec3 b) { return (a == b); }
varying vec3 v_in0;
uniform ivec3 ref_out0;
ivec3 out0;


void main()
{
    bvec3 in0 = bvec3(v_in0);
    out0 = ivec3(in0);
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
