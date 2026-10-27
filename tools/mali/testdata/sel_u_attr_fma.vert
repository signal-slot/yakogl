/* Uniforms (L3 reference): an attribute times a uniform plus another attribute (one fused FMA, two loads before it) (tools/mali/drm_shim_ref.sh run ... --uniform-values, Mesa 26.2.1 on drm-shim). */
attribute vec4 a; attribute vec4 b; uniform vec4 u; void main(){ gl_Position = a * u + b; }
