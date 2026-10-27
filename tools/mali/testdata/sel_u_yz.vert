/* Uniforms (L3 reference): u.yz only: x..z are pushed (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; uniform vec4 u; void main(){ gl_Position = vec4(a_pos.xy + u.yz, 0.0, 1.0); }
