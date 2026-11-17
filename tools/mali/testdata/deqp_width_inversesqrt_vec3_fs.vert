/* Fragment inversesqrt of a mediump vec3: dEQP-GLES2.functional.shaders.
 * operator.exponential.inversesqrt.mediump_vec3_fragment, the Mesa 26.2.1
 * drm-shim reference. */
attribute highp vec4 a_position;
attribute mediump vec4 a_in0;
varying mediump vec4 v_in0;

void main()
{
    gl_Position = a_position;
    v_in0 = a_in0;
}
