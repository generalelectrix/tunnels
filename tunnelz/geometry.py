"""Module to create geometry constants previously delivered by Processing."""
from collections import namedtuple

__all__ = (
    'geometry',
)

Geometry = namedtuple("Geometry",
    ("max_radius", "max_x_offset", "max_y_offset", "x_nudge", "y_nudge", "thickness_scale"))

# TODO: move this to a presets file and share it with dumbtunnel?
geometry = Geometry(
    max_size=1.0, # x screen min dimension
    max_x_offset=0.5, # x screen x_size
    max_y_offset=0.5, # x screen y_size
    x_nudge=0.025, # fraction of half-screen
    y_nudge=0.025, # fraction of half-screen
    thickness_scale=0.5) # half size of screen min dimension

