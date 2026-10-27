/* Uniforms (L3 reference): two vec2 uniforms in one fused multiply-add (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a_pos; uniform vec2 u_e; uniform vec2 u_f; void main(){ gl_Position = vec4(a_pos.xy * u_e + u_f, a_pos.zw); }
