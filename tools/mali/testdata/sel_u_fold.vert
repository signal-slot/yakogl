/* Uniforms (L3 reference): a uniform times 0.0 plus 1.0 folds to 1.0 (the uniform takes no word) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; uniform vec4 u; void main(){ gl_Position = a_pos * (u * 0.0 + 1.0); }
