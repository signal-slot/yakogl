/* dEQP-GLES2.functional.light_amount, its fragment shader
 * (es2fLightAmountTest.cpp, Apache-2.0). */
precision mediump float;
varying highp vec3 v_color;

void main()
{
    gl_FragColor = vec4(v_color, 1.0);
}
