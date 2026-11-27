/* dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_vec4_yyw_vertex (vertex shader),
 * the shader pair the dEQP vertex case generates from VK-GL-CTS 93bca018
 * (data/gles2/shaders/swizzles.test, Apache-2.0), laid out as the .qpa of
 * tools/mali/deqp-host-measure.sh records the swz_mixed pair. Shape swz_mixed
 * of docs/notes/mali-deqp-varylink2.md, a non-aligned swizzle (out0 = in0.yyw). */
precision mediump float;

attribute highp vec4 dEQP_Position;
attribute vec4 in0;
varying vec3 out0;


void main()
{
    
    out0 = in0.yyw;
    gl_Position = dEQP_Position;

}
