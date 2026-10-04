#version 300 es
// gl_test case 104 "divloop": a loop whose trip count differs from pixel to
// pixel, run under the execution mask. Band b of eight (left to right) loops
// b times: band 0 never enters, band 7's bound is past the cap and stops at
// 4096 (UNIFORM_LOOP_CAP, a deliberate deviation). The count is written in
// steps of 16 per channel (red the ones, green the sixteens, blue the 256s);
// a counter carried out of the loop that disagrees with it turns the pixel
// white.
precision highp float;
precision highp int;
in float v_s;
layout(location = 0) out vec4 fragColor;
void main()
{
    float b = floor(v_s * 8.0);
    float bound = b > 6.5 ? 1.0e9 : b;
    float n = 0.0;
    int last = -1;
    for (int i = 0; float(i) < bound; i++) {
        n += 1.0;
        last = i;
    }
    vec3 c = vec3(mod(n, 16.0), mod(floor(n / 16.0), 16.0), min(floor(n / 256.0), 15.0)) * (16.0 / 255.0);
    fragColor = last + 1 == int(n) ? vec4(c, 1.0) : vec4(1.0);
}
