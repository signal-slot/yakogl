#version 100
precision mediump float;
precision mediump int;

bool isOk (float a, int b)  { float atemp = a+0.5; return (float(b) <= atemp && atemp <= float(b+1)); }

varying float v_out0;
uniform int ref_out0;

void main()
{
    bool RES = isOk(v_out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);
}
