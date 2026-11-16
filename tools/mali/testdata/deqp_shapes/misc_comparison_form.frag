#version 100
precision highp float;
precision highp int;
bool isOk (bool a, bool b)   { return (a == b); }
uniform bool ref_out0;
bool out0;

void main()
{
    const bool cval = notEqual(vec2 (1.0), vec2 (1.2)).y;
    out0 = cval;
    bool RES = isOk(out0, ref_out0);
gl_FragColor = vec4(RES, RES, RES, 1.0);

}
