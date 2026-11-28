/* dEQP-GLES2.functional.shaders.swizzles.vector_swizzles.mediump_vec3_xyyx_vertex (vertex shader),
 * the shader pair the dEQP vertex case generates from VK-GL-CTS 93bca018
 * (Apache-2.0), laid out as the .qpa of tools/mali/deqp-host-measure.sh records
 * the neighbouring deqp_varylink2_* pairs. Shape equal-lanes of spike-equal-lanes: out0 = in0.xyyx. */
precision mediump float;

attribute highp vec4 dEQP_Position;
attribute vec3 in0;
varying vec4 out0;


void main()
{
    
    out0 = in0.xyyx;
    gl_Position = dEQP_Position;

}
