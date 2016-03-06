from math import pi

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

    if angle > PI - smoothing:
        return PI/smoothing - angle/smoothing
    elif angle < smoothing - PI:
        return -PI/smoothing - angle/smoothing
    else:
        return angle/(PI - smoothing)