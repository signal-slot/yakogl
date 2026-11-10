/* Motion guard pair (docs/notes/mali-trunc.md, fs-trunc): v is fed by a uniform alone, so its lanes are convergent (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1). */
attribute vec4 p; uniform vec4 u; varying vec4 v; void main(){ gl_Position = p; v = u; }
