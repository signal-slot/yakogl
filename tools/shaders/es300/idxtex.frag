#version 300 es
// gl_test case 100 "idxtex": the fragment stage's texture and dynamically
// indexed uniform array (see idxtex.vert). Blue and alpha carry this stage;
// red and green are the vertex stage's, passed through.
precision highp float;
precision highp int;
in vec4 v_col;
uniform highp sampler2D ftex;
uniform vec4 fcols[4];
uniform int fidx;
layout(location = 0) out vec4 fragColor;
void main()
{
    vec4 f = 0.5 * (fcols[fidx] + texture(ftex, vec2(0.75, 0.75)));
    fragColor = vec4(v_col.rg, f.ba);
}
