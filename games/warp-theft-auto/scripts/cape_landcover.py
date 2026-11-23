"""Land cover baked onto the cape's existing ground triangles (linear RGB, no textures)."""

from math import cos, sin


LAND_COLORS = {
    "Hay meadow": (.222, .233, .115),
    "Coastal heath": (.171, .149, .105),
    "Woodland floor": (.109, .133, .072),
    "Granite ledge": (.262, .257, .223),
}


def ground_material(x, z, height, slope):
    # Long meadow strips give the driver a landmark on either side of the trail.
    weather = sin(.015 * x + .022 * z) + .53 * cos(.034 * z - .010 * x)
    if slope > .315 + .045 * weather or (height > 21 + 2.5 * weather and slope > .13):
        return "Granite ledge"
    if height > 16 + 2 * weather or slope > .21:
        return "Coastal heath"
    # Open farmland on the approach, damp forest understory on the sheltered slopes.
    if ((x + 690) / 150) ** 2 + ((z + 99) / 100) ** 2 < .98 + .10 * weather or (
        ((x + 900) / 95) ** 2 + ((z - 108) / 83) ** 2 < .95 + .12 * weather
    ):
        return "Woodland floor"
    if (x > -550 and weather > -.20) or weather > 1.05:
        return "Hay meadow"
    if weather < -.54:
        return "Grassy hollows"
    return "Rough pasture"
