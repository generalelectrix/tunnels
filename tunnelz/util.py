from math import pi

def unwrap(angle):
    """Unwrap an angle in radians."""
    while pi < angle:
        angle = angle - 2*pi

    while pi < -1*angle:
        angle = angle + 2*pi

    return angle