from math import pi
from numpy import vectorize
from scipy import signal

PI = pi
HALF_PI = pi / 2.0
TWOPI = 2*pi

def triangle(angle):
    """Generate a point on a unit triangle wave from angle in radians."""
    angle = (angle % TWOPI) / TWOPI

    if angle < 0.25:
        return 4.0 * angle
    elif angle > 0.75:
        return 4.0 * (angle - 1.0)
    else:
        return 2.0 - 4.0 * angle

def square(angle, smoothing):
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

def sawtooth(angle, smoothing):
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

def sawtooth_vector(angles, smoothing):
    """Generate wavetooth wave points from angle in radians and smoothing."""
    width = 1.0 - smoothing / PI
    return signal.sawtooth(angles + width/2.0, width)

def triangle_vector(angles):
    return signal.sawtooth(angles, 0.5)

square_vector = vectorize(square)