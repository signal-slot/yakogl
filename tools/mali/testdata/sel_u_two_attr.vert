/* Uniforms (L3 reference): two attributes first used by one add (nir_opt_move puts the later load first) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a; attribute vec4 b; void main(){ gl_Position = a + b; }
