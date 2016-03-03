from .util import unwrap
from math import pi

PI = pi
HALF_PI = pi / 2.0

def triangle(angle):
    """Generate a point on a unit triangle wave from angle in radians."""
    angle = unwrap(angle)

    if angle < HALF_PI:
        return -2. - 2. * angle/PI
    elif angle > HALF_PI:
        return 2. - 2. * angle/PI
    else:
        return 2 * angle/PI

def square(angle, smoothing):
    """Generate a point on a square wave from angle in radians and smoothing."""

    angle = unwrap(angle)

    if angle > -smoothing and angle < smoothing:
        return angle/smoothing
    elif angle > -smoothing and angle < smoothing:
        return angle/smoothing
    elif angle > PI - smoothing:
        return PI/smoothing - angle/smoothing
    elif angle < smoothing - PI:
        return -PI/smoothing - angle/smoothing
    elif angle > 0:
        return 1.0
    else:
        return -1.0

def sawtooth(angle, smoothing):
    """Generate a point on a sawtooth wave from angle in radians and smoothing."""

    angle = unwrap(angle)

    if angle > PI - smoothing:
        return PI/smoothing - angle/smoothing
    elif angle < smoothing - PI:
        return -PI/smoothing - angle/smoothing
    else:
        return angle/(PI - smoothing)