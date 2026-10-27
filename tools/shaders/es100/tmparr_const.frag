#version 100
/* dEQP shaders.indexing.tmp_array.vec4_const_write_dynamic_read_fragment
 * (es2fShaderIndexingTests.cpp createTmpArrayCase, Apache-2.0): a 40-element
 * array of constants read at run-time indices (0..3, as dEQP sets them).
 * About 50 distinct constants live at once unless each is loaded again at
 * its read (select.rs Pressure::Remat). */
precision mediump float;
uniform mediump int ui_zero, ui_one, ui_two, ui_three;

void main()
{
    mediump vec4 arr[40];
    arr[0] = vec4(0.125, 0.25, 0.5, 1.0);
    arr[1] = vec4(0.125, 0.25, 0.5, 1.0) * 0.5;
    arr[2] = vec4(0.125, 0.25, 0.5, 1.0) * 0.25;
    arr[3] = vec4(0.125, 0.25, 0.5, 1.0) * 0.125;
    int i = 4;
    arr[i++] = vec4(4.0);
    arr[i++] = vec4(5.0);
    arr[i++] = vec4(6.0);
    arr[i++] = vec4(7.0);
    arr[i++] = vec4(8.0);
    arr[i++] = vec4(9.0);
    arr[i++] = vec4(10.0);
    arr[i++] = vec4(11.0);
    arr[i++] = vec4(12.0);
    arr[i++] = vec4(13.0);
    arr[i++] = vec4(14.0);
    arr[i++] = vec4(15.0);
    arr[i++] = vec4(16.0);
    arr[i++] = vec4(17.0);
    arr[i++] = vec4(18.0);
    arr[i++] = vec4(19.0);
    arr[i++] = vec4(20.0);
    arr[i++] = vec4(21.0);
    arr[i++] = vec4(22.0);
    arr[i++] = vec4(23.0);
    arr[i++] = vec4(24.0);
    arr[i++] = vec4(25.0);
    arr[i++] = vec4(26.0);
    arr[i++] = vec4(27.0);
    arr[i++] = vec4(28.0);
    arr[i++] = vec4(29.0);
    arr[i++] = vec4(30.0);
    arr[i++] = vec4(31.0);
    arr[i++] = vec4(32.0);
    arr[i++] = vec4(33.0);
    arr[i++] = vec4(34.0);
    arr[i++] = vec4(35.0);
    arr[i++] = vec4(36.0);
    arr[i++] = vec4(37.0);
    arr[i++] = vec4(38.0);
    arr[i++] = vec4(39.0);
    mediump vec4 res = vec4(0.0);
    res += arr[ui_zero];
    res += arr[ui_one];
    res += arr[ui_two];
    res += arr[ui_three];
    gl_FragColor = vec4(res);
}
