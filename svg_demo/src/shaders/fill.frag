#version 150 core

// The ramp is one cycle of the colour waveform, sampled with repeating wrap, so
// the sawtooth's discontinuity lands exactly where it belongs however coarse
// the mesh is and the cycle count never reaches the geometry.
//
// The ramp texture carries no gamma conversion of its own, so a texel arrives
// here as the byte the ramp was built from, and sRGB encoding on the
// framebuffer converts whatever is written on the way out. Both belong to the
// context and the texture rather than to this shader, which is why a fill drawn
// here and one drawn through piston land on the same colour.

uniform sampler2D u_ramp;
uniform int u_flat;
uniform vec4 u_color;

in vec2 v_uv;
in vec4 v_tint;

out vec4 o_Color;

void main() {
    if (u_flat != 0) {
        o_Color = u_color;
    } else {
        o_Color = texture(u_ramp, v_uv) * v_tint;
    }
}
