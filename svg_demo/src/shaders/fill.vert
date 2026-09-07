#version 150 core

// Everything the CPU vertex pass does, per vertex, on the GPU: the polar
// coordinates a vertex is read in, the geometry animations that displace it,
// the coordinate it looks up in the ramp, and the tint a second-axis
// brightness animation rides in on.
//
// The waveforms are ported from tunnels_model, so a look dialled in on the CPU
// path arrives here unchanged. Noise is absent: it reads a simplex field
// indexed by a vertex's position in the mesh, which no uniform can carry, and
// a layer using it draws on the CPU instead.

const float TAU = 6.283185307179586;
const float PI = 3.141592653589793;
const float SQRT_2 = 1.4142135623730951;

// How far a spin animation can rotate a point at full size: a quarter turn.
const float MAX_SPIN = TAU / 4.0;

const int MAX_WAVES = 3;

// Which coordinate a waveform runs along.
const int PHASE_ANGLE = 0;
const int PHASE_RADIUS = 1;
const int PHASE_X = 2;
const int PHASE_Y = 3;

// What a slot drives. Zero is an empty slot.
const int TARGET_NONE = 0;
const int TARGET_RADIAL = 1;
const int TARGET_SPIN = 2;
const int TARGET_ASPECT = 3;
const int TARGET_HUE = 4;
const int TARGET_BRIGHTNESS = 5;

const int WAVE_SINE = 0;
const int WAVE_TRIANGLE = 1;
const int WAVE_SQUARE = 2;
const int WAVE_SAWTOOTH = 3;
const int WAVE_CONSTANT = 4;

const int FLAG_PULSE = 1;
const int FLAG_STANDING = 2;
const int FLAG_INVERT = 4;

// Shape-space position, and the whole turns that put this vertex on the same
// angular branch as the rest of its triangle.
in vec2 a_pos;
in float a_turn;

// The rows of the 2D affine transform. It carries the viewport, so its output
// is already clip space.
uniform vec3 u_xform0;
uniform vec3 u_xform1;

uniform int u_phase;
uniform float u_cycles;
uniform float u_unwrap;
uniform float u_spin;
uniform int u_rotates;

uniform int u_wave_target[MAX_WAVES];
uniform int u_wave_phase[MAX_WAVES];
uniform int u_wave_form[MAX_WAVES];
uniform int u_wave_flags[MAX_WAVES];
uniform float u_wave_periods[MAX_WAVES];
uniform float u_wave_duty[MAX_WAVES];
uniform float u_wave_smoothing[MAX_WAVES];
uniform float u_wave_temporal[MAX_WAVES];
uniform float u_wave_scale[MAX_WAVES];

out vec2 v_uv;
out vec4 v_tint;

// Unit phase along one coordinate, reusing polar coordinates already worked
// out for the vertex.
float phase_along(vec2 p, float radius, float angle, int mode) {
    if (mode == PHASE_ANGLE) {
        return angle / TAU + 0.5;
    }
    if (mode == PHASE_RADIUS) {
        return radius / SQRT_2;
    }
    if (mode == PHASE_X) {
        return (p.x + 1.0) * 0.5;
    }
    return (p.y + 1.0) * 0.5;
}

float sine_spatial(float p, float duty, bool pulse) {
    if (p > duty || duty == 0.0) {
        return 0.0;
    }
    float sp = fract(p / duty);
    if (pulse) {
        return (sin(TAU * sp - PI * 0.5) + 1.0) * 0.5;
    }
    return sin(TAU * sp);
}

float triangle_spatial(float p, float duty, bool pulse) {
    if (p > duty || duty == 0.0) {
        return 0.0;
    }
    float sp = fract(p / duty);
    if (pulse) {
        return sp < 0.5 ? 2.0 * sp : 2.0 * (1.0 - sp);
    }
    if (sp < 0.25) {
        return 4.0 * sp;
    }
    if (sp > 0.75) {
        return 4.0 * (sp - 1.0);
    }
    return 2.0 - 4.0 * sp;
}

float square_spatial(float p, float smoothing, float duty, bool pulse) {
    // Pulse compresses the duty cycle rather than the waveform, so it applies
    // before the phase is tested against it.
    if (pulse) {
        duty *= 0.5;
    }
    if (p > duty || duty == 0.0) {
        return 0.0;
    }
    float sp = fract(p / duty);
    if (pulse) {
        sp = sp * 0.5;
    }
    // The internal smoothing scale runs to a quarter.
    float sm = smoothing * 0.25;
    if (sm == 0.0) {
        return sp < 0.5 ? 1.0 : -1.0;
    }
    if (sp < sm) {
        return sp / sm;
    }
    if (sp > 0.5 - sm && sp < 0.5 + sm) {
        return -(sp - 0.5) / sm;
    }
    if (sp > 1.0 - sm) {
        return (sp - 1.0) / sm;
    }
    if (sp >= sm && sp <= 0.5 - sm) {
        return 1.0;
    }
    return -1.0;
}

float sawtooth_spatial(float p, float smoothing, float duty, bool pulse) {
    if (p > duty || duty == 0.0) {
        return 0.0;
    }
    float sp = fract(p / duty);
    if (pulse) {
        sp = sp * 0.5;
    }
    float sm = smoothing * 0.25;
    if (sm == 0.0) {
        return sp < 0.5 ? 2.0 * sp : 2.0 * (sp - 1.0);
    }
    if (sp < 0.5 - sm) {
        return sp / (0.5 - sm);
    }
    if (sp > 0.5 + sm) {
        return (sp - 1.0) / (0.5 - sm);
    }
    return -(sp - 0.5) / sm;
}

// One animation slot's value at a point in its spatial phase, amplitude and
// all.
float wave_value(int i, float unit_phase) {
    int flags = u_wave_flags[i];
    bool pulse = (flags & FLAG_PULSE) != 0;
    bool standing = (flags & FLAG_STANDING) != 0;
    bool invert = (flags & FLAG_INVERT) != 0;
    int form = u_wave_form[i];

    float value;
    if (form == WAVE_CONSTANT) {
        // A constant is the one waveform with no shape for a standing envelope
        // or a duty cycle to act on.
        value = 1.0;
    } else {
        float p = fract(unit_phase * u_wave_periods[i]);
        float amplitude = 1.0;
        if (standing) {
            // A standing wave modulates in place: the clock drives the
            // envelope rather than sliding the waveform along.
            amplitude = cos(TAU * u_wave_temporal[i]);
            if (pulse) {
                amplitude = (amplitude + 1.0) * 0.5;
            }
        } else {
            p = fract(p + u_wave_temporal[i]);
        }
        float duty = u_wave_duty[i];
        float smoothing = u_wave_smoothing[i];
        float spatial;
        if (form == WAVE_SINE) {
            spatial = sine_spatial(p, duty, pulse);
        } else if (form == WAVE_TRIANGLE) {
            spatial = triangle_spatial(p, duty, pulse);
        } else if (form == WAVE_SQUARE) {
            spatial = square_spatial(p, smoothing, duty, pulse);
        } else {
            spatial = sawtooth_spatial(p, smoothing, duty, pulse);
        }
        value = amplitude * spatial;
    }
    if (invert) {
        value = -value;
    }
    return value * u_wave_scale[i];
}

float srgb_to_linear(float f) {
    return f <= 0.04045 ? f / 12.92 : pow((f + 0.055) / 1.055, 2.4);
}

void main() {
    vec2 p = a_pos;
    float radius = length(p);
    float angle = atan(p.y, p.x);

    float radial = 1.0;
    float turn = u_spin * radius * TAU;
    vec2 stretch = vec2(1.0, 1.0);
    float hue_shift = 0.0;
    float brightness = 1.0;

    for (int i = 0; i < MAX_WAVES; i++) {
        int target = u_wave_target[i];
        if (target == TARGET_NONE) {
            continue;
        }
        float value = wave_value(i, phase_along(p, radius, angle, u_wave_phase[i]));
        if (target == TARGET_RADIAL) {
            // Multiplicative, so the deformation is proportional: a waveform
            // around the angle turns a disc into petals.
            radial *= 1.0 + value;
        } else if (target == TARGET_SPIN) {
            turn += value * MAX_SPIN;
        } else if (target == TARGET_ASPECT) {
            stretch.x *= 1.0 + value;
            stretch.y *= 1.0 - value;
        } else if (target == TARGET_HUE) {
            hue_shift += value;
        } else {
            // Only ever darkens, matching what the ramp does with it.
            brightness *= clamp(1.0 + value, 0.0, 1.0);
        }
    }
    // A negative radius would turn the shape inside out through the origin
    // rather than collapsing it.
    radial = max(radial, 0.0);

    vec2 displaced;
    if (u_rotates != 0) {
        float a = angle + turn;
        float r = radius * radial;
        displaced = vec2(r * cos(a) * stretch.x, r * sin(a) * stretch.y);
    } else {
        // Without a rotation the angle never changes, so scaling the radius is
        // scaling x and y — no round trip through polar coordinates.
        displaced = p * radial * stretch;
    }

    // Phase comes from the undeformed position, so a colour pattern stays glued
    // to the shape while a warp moves it rather than sliding across it.
    float phase = phase_along(p, radius, angle, u_phase) + a_turn * u_unwrap;
    v_uv = vec2(phase * u_cycles + hue_shift, 0.5);
    float tint = srgb_to_linear(brightness);
    v_tint = vec4(tint, tint, tint, 1.0);

    vec3 h = vec3(displaced, 1.0);
    gl_Position = vec4(dot(u_xform0, h), dot(u_xform1, h), 0.0, 1.0);
}
