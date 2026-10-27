/* Uniforms (L3 reference): an attribute times a uniform plus a constant (the fused FMA's addend a table entry) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; uniform vec4 u; void main(){ gl_Position = a_pos * u + vec4(1.0, 2.0, 1.0, 2.0); }
