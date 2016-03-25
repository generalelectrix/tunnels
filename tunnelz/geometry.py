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
    ("max_radius", "max_x_offset", "max_y_offset", "x_nudge", "y_nudge", "thickness_scale"))

# TODO: move this to a presets file and share it with dumbtunnel?
geometry = Geometry(
    max_radius=2.0, # 2x size of screen min dimension
    max_x_offset=1.0, # edge of screen
    max_y_offset=1.0, # edge of screen
    x_nudge=0.025, # fraction of half-screen
    y_nudge=0.025, # fraction of half-screen
    thickness_scale=0.5) # half size of screen min dimension

