/* Uniforms (L3 reference): a vec4 varying times a mediump vec4 uniform (FMA.v2f16 with FAU sources) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
precision mediump float; uniform vec4 u_color; varying vec4 v_color; void main(){ gl_FragColor = v_color * u_color; }
