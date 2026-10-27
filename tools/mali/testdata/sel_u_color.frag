/* Uniforms (L3 reference): a mediump vec4 uniform written whole as the colour (fp16 in the uniform block) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
precision mediump float; uniform vec4 u_color; void main(){ gl_FragColor = u_color; }
