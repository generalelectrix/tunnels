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