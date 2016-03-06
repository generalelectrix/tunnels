from math import pi
from numpy import vectorize

PI = pi
HALF_PI = pi / 2.0
TWOPI = 2*pi

triangle_vector = vectorize(triangle)
square_vector = vectorize(square)
sawtooth_vector = vectorize(sawtooth)

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