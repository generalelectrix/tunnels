"""Definitions of commonly-used waveform functions.

Angles should be passed as floats on the range [0.0, 1.0].  These "radial units"
minimize unneded scalings and rescalings involving pi.  Unifying as many control
parameters as possible to the same scaling significantly lubricates UI.
"""
from __future__ import division
from math import pi
import numpy as np
cimport numpy as np
from cpython cimport bool

cdef float PI = np.pi
cdef float HALF_PI = np.pi / 2.0
cdef float TWOPI = 2*np.pi

cpdef inline sine(float angle, float smoothing, float duty_cycle, bool pulse):
    angle = angle % 1.0

    if angle > duty_cycle or duty_cycle == 0.0:
        return 0.0
    else:
        angle = angle / duty_cycle
        if pulse:
            angle = (angle - HALF_PI)
            return (np.sin(TWOPI * angle) + 1.0) / 2.0
        else:
            return np.sin(TWOPI * angle)

cpdef inline triangle (float angle, float smoothing, float duty_cycle, bool pulse):
    """Generate a point on a unit triangle wave from angle."""
    angle = angle % 1.0

    if angle > duty_cycle or duty_cycle == 0.0:
        return 0.0
    else:
        angle = angle / duty_cycle
        if pulse:
            if angle < 0.5:
                return 2.0 * angle
            else:
                return 2.0 * (1.0 - angle)
        else:
            if angle < 0.25:
                return 4.0 * angle
            elif angle > 0.75:
                return 4.0 * (angle - 1.0)
            else:
                return 2.0 - 4.0 * angle

cpdef inline float square(float angle, float smoothing, float duty_cycle, bool pulse):
    """Generate a point on a square wave from angle and smoothing."""

    angle = angle % 1.0

    if angle > duty_cycle or duty_cycle == 0.0:
        return 0.0
    else:
        angle = angle / duty_cycle
        if pulse:
            return square(angle/2.0, smoothing, 1.0, False)

        else:
            if smoothing == 0.0:
                if angle < 0.5:
                    return 1.0
                else:
                    return -1.0

            if angle < smoothing:
                return angle / smoothing
            elif angle > (0.5 - smoothing) and angle < (0.5 + smoothing):
                return -(angle - 0.5)/smoothing
            elif angle > (1.0 - smoothing):
                return (angle - 1.0)/smoothing
            elif angle >= smoothing and angle <= 0.5 - smoothing:
                return 1.0
            else:
                return -1.0

cpdef inline float sawtooth(float angle, float smoothing, float duty_cycle, bool pulse):
    """Generate a point on a sawtooth wave from angle and smoothing."""

    angle = angle % 1.0

    if angle > duty_cycle or duty_cycle == 0.0:
        return 0.0
    else:
        angle = angle / duty_cycle
        if pulse:
            return sawtooth(angle/2.0, smoothing, 1.0, False)

        if smoothing == 0.0:
            if angle < 0.5:
                return 2.0 * angle
            else:
                return 2.0 * (angle - 1.0)

        if angle < 0.5 - smoothing:
            return angle / (0.5 - smoothing)
        elif angle > 0.5 + smoothing:
            return (angle - 1.0) / (0.5 - smoothing)
        else:
            return -(angle - 0.5)/smoothing

def sine_vector(np.ndarray[np.float_t, ndim=1] angles, np.float smoothing, np.float duty_cycle, bool pulse):
    cdef int size = angles.shape[0]
    cdef np.ndarray[np.float_t, ndim=1] output = np.empty(size, np.float)
    cdef size_t i

    for i in xrange(size):
        output[i] = sine(angles[i], smoothing, duty_cycle, pulse)

    return output

def sawtooth_vector(np.ndarray[np.float_t, ndim=1] angles, np.float smoothing, np.float duty_cycle, bool pulse):
    """Generate wavetooth wave points from angle in radians and smoothing."""
    cdef int size = angles.shape[0]
    cdef np.ndarray[np.float_t, ndim=1] output = np.empty(size, np.float)
    cdef size_t i

    for i in xrange(size):
        output[i] = sawtooth(angles[i], smoothing, duty_cycle, pulse)

    return output

def triangle_vector(np.ndarray[np.float_t, ndim=1] angles, np.float smoothing, np.float duty_cycle, bool pulse):
    cdef int size = angles.shape[0]
    cdef np.ndarray[np.float_t, ndim=1] output = np.empty(size, np.float)
    cdef size_t i

    for i in xrange(size):
        output[i] = triangle(angles[i], smoothing, duty_cycle, pulse)

    return output

def square_vector(np.ndarray[np.float_t, ndim=1] angles, np.float smoothing, np.float duty_cycle, bool pulse):
    cdef int size = angles.shape[0]
    cdef np.ndarray[np.float_t, ndim=1] output = np.empty(size, np.float)
    cdef size_t i

    for i in xrange(size):
        output[i] = square(angles[i], smoothing, duty_cycle, pulse)

    return output

def clamp_to_unit(np.ndarray[np.float_t, ndim=1] vals):
    return np.clip(vals, 0.0, 1.0, vals)