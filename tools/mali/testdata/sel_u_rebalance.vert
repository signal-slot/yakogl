/* Uniforms (L3 reference): a chain of additions inside a constructor: GLSL's do_rebalance_tree reassociates it (refused; the reference shows the tree) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; uniform vec4 u0; uniform vec4 u1; uniform vec4 u2; void main(){ gl_Position = vec4(a_pos.x + u0.x + u1.x + u2.x, a_pos.yzw); }
