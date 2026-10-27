#version 300 es
// gl_test case 104 "divloop", its nested half: a loop inside a loop, both
// bounded per pixel (band b of eight loops b times), with a `continue` and a
// `break` at each level -- the outer skips i == 2 and leaves once n >= 6,
// the inner skips j == 1 and leaves at j >= b - 2. n counts the inner
// passes that reach the end, written as in divloop.frag; alpha is the outer
// loop's passes (the counter carried out of it) in steps of 32.
precision highp float;
precision highp int;
in float v_s;
layout(location = 0) out vec4 fragColor;
void main()
{
    float b = floor(v_s * 8.0);
    float n = 0.0;
    int outer = 0;
    for (int i = 0; float(i) < b; i++) {
        outer = i + 1;
        if (i == 2)
            continue;
        for (int j = 0; float(j) < b; j++) {
            if (j == 1)
                continue;
            if (float(j) >= b - 2.0)
                break;
            n += 1.0;
        }
        if (n >= 6.0)
            break;
    }
    vec3 c = vec3(mod(n, 16.0), mod(floor(n / 16.0), 16.0), min(floor(n / 256.0), 15.0)) * (16.0 / 255.0);
    fragColor = vec4(c, float(outer) * (32.0 / 255.0));
}
