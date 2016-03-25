"""Module to create geometry constants previously delivered by Processing."""
from collections import namedtuple

__all__ = (
    'geometry',
)

ResolutionData = namedtuple(
    'ResolutionData',
    ('x_size', 'y_size', 'x_center', 'y_center', 'max_radius', 'max_x_offset',
    'max_y_offset', 'thickness_scale', 'x_nudge', 'y_nudge'))

def make_resolution_data(x_size, y_size):
    return ResolutionData(
        x_size=x_size,
        y_size=y_size,
        x_center=x_size/2,
        y_center=y_size/2,
        max_radius=min(x_size, y_size)/2,
        max_x_offset=x_size/2,
        max_y_offset=y_size/2,
        thickness_scale=0.27*x_size, # implies that maximum thickness is 0.27*x_size
        x_nudge=(10*x_size)/1280,
        y_nudge=(10*x_size)/1280,)

Geometry = namedtuple("Geometry",
    ("max_radius", "max_x_offset", "max_y_offset", "x_nudge", "y_nudge"))

geometry = Geometry(
    max_radius=2.0,
    max_x_offset=1.0,
    max_y_offset=1.0,
    x_nudge=0.025,
    y_nudge=0.025)

