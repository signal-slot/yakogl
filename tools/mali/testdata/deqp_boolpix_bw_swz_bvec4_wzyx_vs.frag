#version 100
precision mediump float;
precision mediump int;

bool isOk (vec4 a, bvec4 b) { return (greaterThan(a, vec4(0.5)) == b); }

varying vec4 v_out0;
uniform bvec4 ref_out0;

void main()
{
    bool RES = isOk(v_out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);
}
