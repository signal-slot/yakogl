/* Uniforms (L3 reference): a float then a vec4: Mesa packs the block (the vec4 at word 1) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; uniform float u_a; uniform vec4 u_b; void main(){ gl_Position = a_pos * u_a + u_b; }
