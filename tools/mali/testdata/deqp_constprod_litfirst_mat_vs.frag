#version 100
precision mediump float;
precision mediump int;

bool isOk (mat2 a, mat2 b, float eps) { vec2 diff = max(abs(a[0]-b[0]), abs(a[1]-b[1])); return all(lessThanEqual(diff, vec2(eps))); }

varying mat2 out0;
uniform mat2 ref_out0;

void main()
{
    bool RES = isOk(out0, ref_out0, 0.05);
gl_FragColor = vec4(RES, RES, RES, 1.0);
}
