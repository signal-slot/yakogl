#version 100
/* dEQP-GLES2.functional.attribute_location, as VK-GL-CTS
 * modules/glshared/glsAttributeLocationTests.cpp generates it (Apache-2.0);
 * reproduced by v3d-glue (.claude/worktrees/deqp-attrib-repro/fragment.frag). */

varying mediump vec4 v_color;
void main (void)
{
	gl_FragColor = v_color;
}
