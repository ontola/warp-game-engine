"""Bake the seeded wet blood footprints used by ground and wall decals."""
from pathlib import Path

import numpy as np
from PIL import Image

SIZE = 256
COLS = 4
GRID = (np.arange(SIZE) + 0.5) * 2 / SIZE - 1
X, Y = np.meshgrid(GRID, GRID)


def noise(rng, cells):
    field = Image.fromarray(rng.integers(0, 256, (cells, cells), dtype=np.uint8))
    return np.asarray(field.resize((SIZE, SIZE), Image.Resampling.BICUBIC)) / 255.0 - 0.5


def splat(seed):
    rng = np.random.default_rng(seed)
    broad = noise(rng, 11)
    medium = noise(rng, 35)
    grit = noise(rng, 110)
    u = X + 0.10 * broad + 0.024 * medium + 0.007 * grit
    v = Y + 0.115 * noise(rng, 10) + 0.023 * noise(rng, 38)
    wet = np.zeros_like(X)

    def deposit(cx, cy, rx, ry, angle, depth):
        # A liquid spot dries to a ragged thin perimeter around a denser core.
        c, s = np.cos(angle), np.sin(angle)
        dx, dy = u - cx, v - cy
        distance = ((dx*c + dy*s)/rx)**2 + ((dy*c - dx*s)/ry)**2
        wet[:] += np.maximum(1 - distance, 0) ** 0.48 * depth

    # Asymmetric overlapping lobes, with a different balance of pooling and spray per tile.
    width = rng.uniform(0.13, 0.32)
    center = rng.uniform(-0.31, -0.12)
    for _ in range(rng.integers(9, 18)):
        cx, cy = rng.normal(center, 0.095), rng.normal(0, width*0.47)
        deposit(cx, cy, rng.uniform(0.12, 0.32), rng.uniform(0.085, width), rng.normal(0, 0.5), rng.uniform(0.38, 0.98))

    for _ in range(rng.integers(6, 13)):
        angle = rng.uniform(-1.05, 1.05)
        length = rng.uniform(0.26, 0.94)
        start_x, start_y = rng.normal(center, 0.07), rng.normal(0, width*0.47)
        curve = rng.normal(0, 0.17)
        base = rng.uniform(0.017, 0.05)
        for k in range(10):
            t = k / 10
            if t > 0.22 and rng.random() < 0.22 + t*0.35:
                continue
            a = angle + t*curve
            cx = start_x + np.cos(a)*length*t
            cy = start_y + np.sin(a)*length*t
            deposit(cx, cy, rng.uniform(0.035, 0.12)*(1-t*0.47), base*(1-t*0.65), a, rng.uniform(0.25, 0.75))

    for _ in range(rng.integers(115, 185)):
        angle = rng.uniform(-np.pi, np.pi) if rng.random() < 0.36 else rng.normal(0, 0.67)
        distance = rng.uniform(0.18, 1.22)
        cx = center + np.cos(angle)*distance
        cy = np.sin(angle)*distance*rng.uniform(0.62, 1.2)
        radius = rng.uniform(0.006, 0.045) ** 1.04
        deposit(cx, cy, radius*rng.uniform(1.1, 2.4), radius, angle, rng.uniform(0.18, 0.94))

    grain = rng.random(wet.shape)
    erosion = 0.11 + 0.33 * medium + 0.37 * grit + 0.11 * grain
    # Stochastic breaks in very thin film reveal the receiving stone's own material.
    coverage = np.clip((wet - erosion)*13, 0, 1)
    coverage *= np.clip((0.94-np.hypot(X, Y))*35, 0, 1)
    pools = np.clip(wet * 0.61 + broad*0.44, 0, 1)
    fresh = np.array([128., 16., 29.])
    deep = np.array([60., 7., 17.])
    rgb = (fresh + (deep-fresh)*pools[..., None]) * (0.94 + 0.16*medium + 0.10*grit)[..., None]
    rgb[coverage < 1/255] = fresh
    return Image.fromarray(np.uint8(np.dstack((rgb, coverage*255)).clip(0,255)))


if __name__ == "__main__":
    atlas = Image.new("RGBA", (SIZE*COLS, SIZE*COLS))
    for i in range(COLS*COLS):
        atlas.paste(splat(7727 + i*137), ((i%COLS)*SIZE, (i//COLS)*SIZE))
    atlas.save(Path(__file__).resolve().parents[1] / "assets" / "blood-splats.png", optimize=True)
