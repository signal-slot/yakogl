#version 100
precision mediump float;
precision mediump int;

bool isOk (vec4 a, vec4 b, float eps) { return all(lessThanEqual(abs(a-b), (eps*abs(b) + eps))); }

varying vec4 v_out0;
uniform vec4 ref_out0;

void main()
{
    bool RES = isOk(v_out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);
}
