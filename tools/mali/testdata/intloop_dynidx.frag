#version 100
/* dEQP shaders.indexing.vector_subscript.vec4_dynamic_subscript_write_
 * dynamic_subscript_read_fragment (es2fShaderIndexingTests.cpp, Apache-2.0):
 * a vector never initialised, written lane by lane at indices read from
 * uniforms (0, 1, 2, 3, as dEQP sets them), then read the same way. */
precision mediump float;
varying mediump vec4 v_coords;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;
void main() {
    mediump vec4 coords = vec4(v_coords);
    mediump vec4 tmp;
    tmp[ui_zero] = coords.x;
    tmp[ui_one] = coords.y * 0.5;
    tmp[ui_two] = coords.z * 0.25;
    tmp[ui_three] = coords.w * 0.125;
    mediump float res = 0.0;
    res += tmp[ui_zero];
    res += tmp[ui_one] * 10.0;
    res += tmp[ui_two] * 100.0;
    res += tmp[ui_three] * 1000.0;
    gl_FragColor = vec4(vec3(res), 1.0);
}
