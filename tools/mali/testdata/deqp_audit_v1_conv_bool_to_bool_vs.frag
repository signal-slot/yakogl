#version 100
precision mediump float;
precision mediump int;

bool isOk (float a, bool b)  { return (bool(a) == b); }

varying float v_out0;
uniform bool ref_out0;

void main()
{
    bool RES = isOk(v_out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);
}
