#version 300 es
/* Remainder and the unsigned forms on data (v3d back end: nir_lower_idiv).
 * GLSL ES 3.00 leaves % with a negative operand undefined, so the signed
 * remainder is taken of non-negative operands; the unsigned ones reach past
 * 2^31. */
precision highp float;
precision highp int;
in vec4 v_color;
out vec4 o_color;
void main() {
    vec4 c = clamp(v_color, 0.0, 1.0); /* every conversion below stays defined */
    int i = int(c.r * 1000000.0);
    int j = int(c.g * 9.0) + 1;
    uint u = uint(c.b * 4000000000.0);
    uint w = uint(c.a * 60000.0) + 1u;
    /* Dividends at the top of the u32 range with divisors that need the
     * second refinement step (found by a model of the sequence with an exact
     * reciprocal; below 2^31 it never fires): 2^32 - 1, - 2, - 3 over 65537,
     * 2^31 - 1 and 2^25 - 1 give 65535/65534/65534, 2/2/1 and 128. */
    uint top = 4294967295u - uint(c.r * 2.99);
    float steps = (float(top / 65537u) - 65534.0) + (float(top / 2147483647u) - 1.0) * 0.5
                  + (float(top / 33554431u) - 128.0) * 0.25;
    o_color = vec4(float(i % j) / 10.0, float(u / w) / 4000000000.0, float(u % w) / 60000.0,
                   float(u / 3u) / 2000000000.0 + steps);
}
