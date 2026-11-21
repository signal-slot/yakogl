#version 100
precision mediump float;
precision mediump int;

bool isOk (vec2 a, bvec2 b) { return (bvec2(a) == b); }

varying vec2 out0;
uniform bvec2 ref_out0;

void main()
{
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);
}
