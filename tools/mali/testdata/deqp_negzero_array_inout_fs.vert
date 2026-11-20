#version 100
/* dEQP-GLES2.functional.shaders.functions.array_arguments.copy_local_inout_on_call_fragment: the vertex shader,
 * verbatim from the .qpa of tools/mali/deqp-host-measure.sh (VK-GL-CTS
 * 93bca018, Apache-2.0; HOST-ONLY run at cc89a1cf). Shape array_inout of
 * docs/notes/mali-deqp-negzero.md. */
precision highp float;
precision highp int;

attribute highp vec4 dEQP_Position;
attribute vec4 a_in0;
varying vec4 in0;

void main()
{
    gl_Position = dEQP_Position;
    in0 = a_in0;
}
