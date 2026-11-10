/* Motion guard pair (docs/notes/mali-trunc.md, fs-trunc): v is fed by an attribute, so its lanes are interpolated (tools/mali/drm_shim_ref.sh run, Mesa 26.2.1). */
attribute vec4 p; attribute vec4 c; varying vec4 v; void main(){ gl_Position = p; v = c; }
