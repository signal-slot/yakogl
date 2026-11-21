#version 100
/* dEQP-GLES2.functional.attribute_location.bind_aliasing.max_cond_float: the fragment shader, verbatim from the .qpa of
 * tools/mali/deqp-host-measure.sh (VK-GL-CTS 93bca018, Apache-2.0;
 * HOST-ONLY run at f9b37651). Drawn with the bindings of the .bind file.
 * Defect `encode` of docs/notes/mali-deqp-defects.md. */

varying mediump vec4 v_color;
void main (void)
{
	gl_FragColor = v_color;
}
