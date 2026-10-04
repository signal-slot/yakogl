#version 300 es
precision highp float;
layout(location = 0) in vec4 pos;
layout(location = 1) in vec2 offs;
layout(location = 2) in vec4 icol;
out vec4 v_col;
void main()
{
    /* Instanced without gl_InstanceID: everything per instance comes from the
     * per-instance attributes, the way Qt Quick 3D's instanced materials read
     * their transform. A draw like this one the backend may split into
     * chunks of instances. */
    gl_Position = vec4(pos.xy * 0.25 + offs, 0.0, 1.0);
    v_col = icol;
}
