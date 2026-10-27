/* Uniforms (L3 reference): u_e.x and u_f.y: two loads, a word between them not pushed (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; uniform vec2 u_e; uniform vec2 u_f; void main(){ gl_Position = vec4(a_pos.x * u_e.x, a_pos.y * u_f.y, 0.0, 1.0); }
