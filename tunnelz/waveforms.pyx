from __future__ import division
from math import pi
import numpy as np
cimport numpy as np
from scipy import signal

cdef float PI = np.pi
cdef float HALF_PI = np.pi / 2.0
cdef float TWOPI = 2*np.pi

cpdef inline triangle (float angle):
    """Generate a point on a unit triangle wave from angle in radians."""
    angle = (angle % TWOPI) / TWOPI

    if angle < 0.25:
        return 4.0 * angle
    elif angle > 0.75:
        return 4.0 * (angle - 1.0)
    else:
        return 2.0 - 4.0 * angle

cpdef inline float square(float angle, float smoothing):
    """Generate a point on a square wave from angle in radians and smoothing."""

    angle = (angle % TWOPI)

    if smoothing == 0.0:
        if angle < PI:
            return 1.0
        else:
            return 1.0

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

cpdef inline float sawtooth(float angle, float smoothing):
    """Generate a point on a sawtooth wave from angle in radians and smoothing."""

    angle = (angle % TWOPI)

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

def sawtooth_vector(np.ndarray[np.float_t, ndim=1] angles, np.float smoothing):
    """Generate wavetooth wave points from angle in radians and smoothing."""
    cdef int size = angles.shape[0]
    cdef np.ndarray[np.float_t, ndim=1] output = np.empty(size, np.float)
    cdef size_t i

    for i in xrange(size):
        output[i] = sawtooth(angles[i], smoothing)

    return output

def triangle_vector(np.ndarray[np.float_t, ndim=1] angles, np.float smoothing):
    cdef int size = angles.shape[0]
    cdef np.ndarray[np.float_t, ndim=1] output = np.empty(size, np.float)
    cdef size_t i

    for i in xrange(size):
        output[i] = triangle(angles[i])

    return output

def square_vector(np.ndarray[np.float_t, ndim=1] angles, np.float smoothing):
    cdef int size = angles.shape[0]
    cdef np.ndarray[np.float_t, ndim=1] output = np.empty(size, np.float)
    cdef size_t i

    for i in xrange(size):
        output[i] = square(angles[i], smoothing)

    return output