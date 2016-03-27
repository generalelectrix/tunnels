from __future__ import division
from math import pi
import numpy as np
cimport numpy as np
from cpython cimport bool

cdef float PI = np.pi
cdef float HALF_PI = np.pi / 2.0
cdef float TWOPI = 2*np.pi

cpdef inline sine(float angle, float smoothing, float duty_cycle, bool pulse):
    angle = (angle % TWOPI)

    if angle > duty_cycle * TWOPI or duty_cycle == 0.0:
        return 0.0
    else:
        angle = angle / duty_cycle
        if pulse:
            angle = (angle - HALF_PI)
            return (np.sin(angle) + 1.0) / 2.0
        else:
            return np.sin(angle)

cpdef inline triangle (float angle, float smoothing, float duty_cycle, bool pulse):
    """Generate a point on a unit triangle wave from angle in radians."""
    angle = (angle % TWOPI) / TWOPI

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
    """Generate a point on a square wave from angle in radians and smoothing."""

    angle = (angle % TWOPI)

    if angle > duty_cycle * TWOPI or duty_cycle == 0.0:
        return 0.0
    else:
        angle = angle / duty_cycle
        if pulse:
            return square(angle/2.0, smoothing, 1.0, False)

        else:
            if smoothing == 0.0:
                if angle < PI:
                    return 1.0
                else:
                    return -1.0

            if angle < smoothing:
                return angle/smoothing
            elif angle > (PI - smoothing) and angle < (PI + smoothing):
                return -(angle - PI)/smoothing
            elif angle > (TWOPI - smoothing):
                return (angle - TWOPI)/smoothing
            elif angle >= smoothing and angle <= PI - smoothing:
                return 1.0
            else:
                return -1.0

cpdef inline float sawtooth(float angle, float smoothing, float duty_cycle, bool pulse):
    """Generate a point on a sawtooth wave from angle in radians and smoothing."""

    angle = (angle % TWOPI)

    if angle > duty_cycle * TWOPI or duty_cycle == 0.0:
        return 0.0
    else:
        angle = angle / duty_cycle
        if pulse:
            return sawtooth(angle/2.0, smoothing, 1.0, False)

        if smoothing == 0.0:
            if angle < PI:
                return angle / PI
            else:
                return (angle - TWOPI) / PI

        if angle < PI - smoothing:
            return angle / (PI - smoothing)
        elif angle > PI + smoothing:
            return (angle - TWOPI) / (PI - smoothing)
        else:
            return -(angle - PI)/smoothing

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