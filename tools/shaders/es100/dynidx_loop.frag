#version 100
/* dEQP shaders.indexing.vector_subscript.vec4_static_loop_subscript_write_
 * dynamic_loop_subscript_read_fragment (es2fShaderIndexingTests.cpp,
 * Apache-2.0): written in a constant loop, read in a loop whose bound is a
 * uniform (4, as dEQP sets it; a counted loop here). */
precision mediump float;
varying mediump vec4 v_coords;
uniform mediump int ui_four;
void main() {
    mediump vec4 coords = vec4(v_coords);
    mediump vec4 tmp;
    for (int i = 0; i < 4; i++) {
        tmp[i] = coords.x;
        coords = coords.yzwx * 0.5;
    }
    mediump float res = 0.0;
    for (int i = 0; i < ui_four; i++)
        res += tmp[i] * float(i + 1);
    gl_FragColor = vec4(vec3(res), 1.0);
}
