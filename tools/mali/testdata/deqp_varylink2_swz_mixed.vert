/* dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_vec2_grg_vertex (vertex shader),
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh at cc89a1cf
 * (VK-GL-CTS 93bca018, Apache-2.0). Shape swz_mixed of
 * docs/notes/mali-deqp-varylink2.md: equal lanes beside a distinct lane (out0 = in0.grg: lanes 0 and 2 equal), refused at the link. */
precision mediump float;

attribute highp vec4 dEQP_Position;
attribute vec2 in0;
varying vec3 out0;


void main()
{
    
    out0 = in0.grg;
    gl_Position = dEQP_Position;

}
