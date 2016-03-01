
def copy_list_of_beams(to_copy):
  """Deep copy a list of beams."""
  return [beam.copy() for beam in to_copy]

def unwrap(angle):
  """Unwrap an angle in radians."""
  while PI < angle:
    angle = angle - TWO_PI

  while PI < -1*angle:
    angle = angle + TWO_PI

  return angle