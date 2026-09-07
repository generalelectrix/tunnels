#version 150 core

// The ramp is one cycle of the colour waveform, sampled with repeating wrap, so
// the sawtooth's discontinuity lands exactly where it belongs however coarse
// the mesh is and the cycle count never reaches the geometry.
//
// The texture is stored as sRGB and the framebuffer encodes on write, so the
// values that arrive here are linear and the ones written out are converted
// back — the same round trip a piston-drawn layer makes.

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
