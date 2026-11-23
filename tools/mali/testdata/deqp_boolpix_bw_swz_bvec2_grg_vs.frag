#version 100
precision mediump float;
precision mediump int;

bool isOk (vec3 a, bvec3 b) { return (greaterThan(a, vec3(0.5)) == b); }

varying vec3 v_out0;
uniform bvec3 ref_out0;

void main()
{
    bool RES = isOk(v_out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);
}
