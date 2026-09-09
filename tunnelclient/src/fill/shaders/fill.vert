#version 150 core

// The per-vertex pass, on the GPU: the polar coordinates a vertex is read in,
// the animations that displace it, the coordinate it looks up in the ramp, and
// the projection into clip space.
//
// The waveforms are ported from `tunnels_model::waveforms` and read the same
// parameters, so a look dialled in on the CPU path arrives here unchanged.
// `Phase` wraps by euclidean modulus, which for a finite value is `fract`, and
// every place the CPU builds one this does the same.
//
// Colour is absent. Every colour animation is resolved into the ramp texture
// before a figure is drawn, once per texel, so nothing about colour varies per
// vertex beyond where in that texture a vertex looks.
//
// Noise is absent. It reads a field a uniform cannot carry, and a layer using
// it draws through the CPU path instead.

const float TAU = 6.283185307179586;
const float PI = 3.141592653589793;
const float HALF_PI = 1.5707963267948966;
const float SQRT_2 = 1.4142135623730951;

// How far a spin animation can turn a point at the rim: a quarter turn.
const float MAX_SPIN = TAU / 4.0;

// Animation slots a beam carries, matching `tunnels_model::tunnel::N_ANIM`.
const int MAX_WAVES = 4;

// Which coordinate of the figure the phase is read along.
const int PHASE_ANGLE = 0;
const int PHASE_RADIUS = 1;
const int PHASE_LINEAR = 2;

// What a slot drives. Zero is a slot the layer does not fill, or one whose
// target the geometry does not answer.
const int TARGET_NONE = 0;
const int TARGET_SIZE = 1;
const int TARGET_SPIN = 2;
const int TARGET_ASPECT = 3;
const int TARGET_POSITION_X = 4;
const int TARGET_POSITION_Y = 5;

const int WAVE_SINE = 0;
const int WAVE_TRIANGLE = 1;
const int WAVE_SQUARE = 2;
const int WAVE_SAWTOOTH = 3;
const int WAVE_CONSTANT = 4;

const int FLAG_PULSE = 1;
const int FLAG_STANDING = 2;
const int FLAG_INVERT = 4;

// The figure-space point, and the whole turns that put this vertex on the same
// angular branch as the rest of its triangle.
in vec2 a_pos;
in float a_turn;

// The rows of the 2D affine transform. It carries the viewport, so what comes
// out of it is already clip space.
uniform vec3 u_xform0;
uniform vec3 u_xform1;

uniform int u_phase;
// What the figure coordinate is stretched by to index the ramp.
uniform float u_ramp_scale;
// The period the baked branch shift is measured in, or zero where the
// coordinate does not wrap.
uniform float u_unwrap;
uniform float u_spin_speed;
// Whether anything turns a point, which is the only thing that makes the round
// trip through polar coordinates worth taking.
uniform int u_rotates;
// Whether the angle and the radius are read at all. A coordinate nothing reads
// is held at zero, which is what the CPU path does with it, and what a phase
// taken along the other coordinate then sees.
uniform int u_needs_angle;
uniform int u_needs_radius;
// The offset a figure's placement carries, read where the figure has no
// coordinate: at the start of the cycle and at the centre. A point's own offset
// is measured from it, so a figure that varies nowhere stays where it was put.
uniform vec2 u_anchor;

uniform int u_wave_target[MAX_WAVES];
uniform int u_wave_form[MAX_WAVES];
uniform int u_wave_flags[MAX_WAVES];
uniform float u_wave_periods[MAX_WAVES];
uniform float u_wave_duty[MAX_WAVES];
uniform float u_wave_smoothing[MAX_WAVES];
uniform float u_wave_temporal[MAX_WAVES];
uniform float u_wave_scale[MAX_WAVES];

out vec2 v_uv;

// Unit phase along one coordinate, reusing what has already been worked out.
float phase_along(vec2 p, float radius, float angle, int axis) {
    if (axis == PHASE_ANGLE) {
        return angle / TAU + 0.5;
    }
    if (axis == PHASE_RADIUS) {
        return radius / SQRT_2;
    }
    return (p.y + 1.0) * 0.5;
}

bool outside_duty_cycle(float phase, float duty) {
    return phase > duty || duty == 0.0;
}

float sine_spatial(float phase, float duty, bool pulse) {
    if (outside_duty_cycle(phase, duty)) {
        return 0.0;
    }
    float p = fract(phase / duty);
    if (pulse) {
        return (sin(TAU * p - HALF_PI) + 1.0) * 0.5;
    }
    return sin(TAU * p);
}

float triangle_spatial(float phase, float duty, bool pulse) {
    if (outside_duty_cycle(phase, duty)) {
        return 0.0;
    }
    float p = fract(phase / duty);
    if (pulse) {
        return p < 0.5 ? 2.0 * p : 2.0 * (1.0 - p);
    }
    if (p < 0.25) {
        return 4.0 * p;
    }
    if (p > 0.75) {
        return 4.0 * (p - 1.0);
    }
    return 2.0 - 4.0 * p;
}

// The bipolar square wave, at a phase already scaled to its duty cycle.
float square_bipolar(float p, float smoothing) {
    // The internal smoothing scale runs to a quarter.
    float sm = smoothing * 0.25;
    if (sm == 0.0) {
        return p < 0.5 ? 1.0 : -1.0;
    }
    if (p < sm) {
        return p / sm;
    }
    if (p > 0.5 - sm && p < 0.5 + sm) {
        return -(p - 0.5) / sm;
    }
    if (p > 1.0 - sm) {
        return (p - 1.0) / sm;
    }
    if (p >= sm && p <= 0.5 - sm) {
        return 1.0;
    }
    return -1.0;
}

float square_spatial(float phase, float smoothing, float duty, bool pulse) {
    if (outside_duty_cycle(phase, duty)) {
        return 0.0;
    }
    float p = fract(phase / duty);
    if (pulse) {
        // Rescaled from the bipolar wave, read three quarters of a cycle ahead
        // so the pulse starts at the bottom of the range and peaks halfway
        // through it, where a sine or triangle pulse peaks.
        return (square_bipolar(fract(p + 0.75), smoothing) + 1.0) * 0.5;
    }
    return square_bipolar(p, smoothing);
}

// The sawtooth at a phase already scaled to its duty cycle.
float sawtooth_bipolar(float p, float smoothing) {
    float sm = smoothing * 0.25;
    if (sm == 0.0) {
        return p < 0.5 ? 2.0 * p : 2.0 * (p - 1.0);
    }
    if (p < 0.5 - sm) {
        return p / (0.5 - sm);
    }
    if (p > 0.5 + sm) {
        return (p - 1.0) / (0.5 - sm);
    }
    return -(p - 0.5) / sm;
}

float sawtooth_spatial(float phase, float smoothing, float duty, bool pulse) {
    if (outside_duty_cycle(phase, duty)) {
        return 0.0;
    }
    float p = fract(phase / duty);
    if (pulse) {
        // Half the phase against a full duty cycle, which reads the rising
        // half of the wave across the whole of the period.
        return sawtooth_bipolar(p * 0.5, smoothing);
    }
    return sawtooth_bipolar(p, smoothing);
}

// One slot's value at a point, amplitude and all.
float wave_value(int i, float unit_phase) {
    int flags = u_wave_flags[i];
    bool pulse = (flags & FLAG_PULSE) != 0;
    bool standing = (flags & FLAG_STANDING) != 0;
    bool invert = (flags & FLAG_INVERT) != 0;
    int form = u_wave_form[i];

    float value;
    if (form == WAVE_CONSTANT) {
        // A constant answers one number, with no shape for a standing
        // envelope or a duty cycle to act on.
        value = 1.0;
    } else {
        float phase = fract(unit_phase * u_wave_periods[i]);
        float amplitude = 1.0;
        if (standing) {
            // A standing wave modulates in place: the clock drives the
            // envelope rather than sliding the waveform along.
            amplitude = cos(TAU * u_wave_temporal[i]);
            if (pulse) {
                amplitude = (amplitude + 1.0) * 0.5;
            }
        } else {
            phase = fract(phase + u_wave_temporal[i]);
        }
        float duty = u_wave_duty[i];
        float smoothing = u_wave_smoothing[i];
        float spatial;
        if (form == WAVE_SINE) {
            spatial = sine_spatial(phase, duty, pulse);
        } else if (form == WAVE_TRIANGLE) {
            spatial = triangle_spatial(phase, duty, pulse);
        } else if (form == WAVE_SQUARE) {
            spatial = square_spatial(phase, smoothing, duty, pulse);
        } else {
            spatial = sawtooth_spatial(phase, smoothing, duty, pulse);
        }
        value = amplitude * spatial;
    }
    if (invert) {
        value = -value;
    }
    return value * u_wave_scale[i];
}

// What a point's animations work out to: a scale about the origin, a turn, a
// squash, and an offset.
struct Displacement {
    float radial;
    float turn;
    vec2 stretch;
    vec2 offset;
};

Displacement displacement_at(vec2 p, float radius, float angle, float along) {
    Displacement d;
    d.radial = 1.0;
    // A figure reads the spin knob as the winding itself, measured in turns at
    // the rim, rather than as a rate integrated into an angle.
    d.turn = u_spin_speed * radius * TAU;
    d.stretch = vec2(1.0, 1.0);
    d.offset = vec2(0.0, 0.0);

    for (int i = 0; i < MAX_WAVES; i++) {
        int target = u_wave_target[i];
        if (target == TARGET_NONE) {
            continue;
        }
        float value = wave_value(i, along);
        if (target == TARGET_SIZE) {
            // Multiplicative, so the deformation is proportional: a waveform
            // around the angle turns a disc into petals.
            d.radial *= 1.0 + value;
        } else if (target == TARGET_SPIN) {
            d.turn += value * MAX_SPIN;
        } else if (target == TARGET_ASPECT) {
            d.stretch.x *= 1.0 + value;
            d.stretch.y *= 1.0 - value;
        } else if (target == TARGET_POSITION_X) {
            // Additive, in the figure's own units.
            d.offset.x += value;
        } else {
            d.offset.y += value;
        }
    }
    // A negative radius would turn the figure inside out through the origin
    // rather than collapsing it.
    d.radial = max(d.radial, 0.0);
    return d;
}

void main() {
    vec2 p = a_pos;
    float radius = u_needs_radius != 0 ? length(p) : 0.0;
    float angle = u_needs_angle != 0 ? atan(p.y, p.x) : 0.0;
    float along = phase_along(p, radius, angle, u_phase);

    Displacement d = displacement_at(p, radius, angle, along);
    vec2 offset = d.offset - u_anchor;

    vec2 moved;
    if (u_rotates != 0) {
        // The figure's own points, moved in polar terms because a spin is a
        // rotation about the same centre the phase is measured from.
        float a = angle + d.turn;
        float r = radius * d.radial;
        moved = vec2(r * cos(a), r * sin(a));
    } else {
        // Without a rotation the angle never changes, so scaling the radius is
        // scaling x and y — no round trip through polar coordinates.
        moved = p * d.radial;
    }
    // The squash multiplies and the offset adds, in that order, so a figure
    // flattened onto one axis is still moved the same distance along it.
    vec2 placed = moved * d.stretch + offset;

    // Phase comes from the undeformed position, so a colour pattern stays glued
    // to the figure while a warp moves it rather than sliding across it.
    v_uv = vec2(along * u_ramp_scale + a_turn * u_unwrap, 0.5);

    vec3 h = vec3(placed, 1.0);
    gl_Position = vec4(dot(u_xform0, h), dot(u_xform1, h), 0.0, 1.0);
}
