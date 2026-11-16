/* dEQP-GLES2.functional.shaders.discard.basic_dynamic, rebuilt from
 * VK-GL-CTS 93bca018 (Apache-2.0): es2fShaderDiscardTests.cpp:218-227
 * (DISCARDTEMPLATE_MAIN_BASIC) with :392-393 (DISCARDMODE_DYNAMIC). It reads
 * v_coords.xy beside v_color, so the link merges lanes across the two varyings. */
varying mediump vec4 v_color;
varying mediump vec4 v_coords;
uniform sampler2D    ut_brick;
uniform mediump int  ui_one;

void main (void)
{
    gl_FragColor = v_color;
    if (v_coords.x+v_coords.y > 0.0) discard;
}
