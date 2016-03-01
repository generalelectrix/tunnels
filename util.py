def unwrap(angle):
    """Unwrap an angle in radians."""
    while PI < angle:
        angle = angle - TWO_PI

    while PI < -1*angle:
        angle = angle + TWO_PI

    return angle