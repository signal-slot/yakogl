#version 300 es
// gl_test case 98 "uniloop": a loop bounded by a uniform (Qt Quick 3D's
// light-loop shape). The count is written in steps of 16 per channel so a
// tolerance of 3 still tells every count apart: red the ones, green the
// sixteens, blue the 256s (at most 15). The counter carried to the end must
// agree with the count, or the pixel is white.
precision highp float;
precision highp int;
uniform float count;
layout(location = 0) out vec4 fragColor;
void main()
{
    float n = 0.0;
    int last = -1;
    for (int i = 0; float(i) < count; i++) {
        n += 1.0;
        last = i;
    }
    vec3 c = vec3(mod(n, 16.0), mod(floor(n / 16.0), 16.0), min(floor(n / 256.0), 15.0)) * (16.0 / 255.0);
    fragColor = last + 1 == int(n) ? vec4(c, 1.0) : vec4(1.0);
}
