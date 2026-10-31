#!/usr/bin/env python3
"""Bake Warp's original explosion smoke into a compact animated density volume.

Requires numpy and scipy; Pillow is only used for optional diagnostic renders.
No game physics, GPU, VDB reader, Blender, or third-party source assets are used.
The solver is an artistic, incompressible fluid approximation: semi-Lagrangian
velocity transport, FFT pressure projection, buoyancy, vorticity confinement,
and bounded MacCormack density transport. It is not a combustion simulator.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
import scipy
from scipy import fft, ndimage

N = 72
SIZE = 48
FRAMES = 32
DURATION = 2.6
SEED = 1871031
LIGHT = np.array([0.4, 1.0, -0.5], dtype=np.float32)  # array axes z,y,x
LIGHT /= np.linalg.norm(LIGHT)
EXTINCTION = 10.0  # per unit of texture coordinates (cube has width one)


def smooth(a, b, x):
    t = np.clip((x - a) / (b - a), 0, 1)
    return t * t * (3 - 2 * t)


def noise(rng, shape, sigma):
    v = ndimage.gaussian_filter(rng.standard_normal(shape).astype(np.float32), sigma, mode="wrap")
    return v / max(float(v.std()), 1e-5)


def project(velocity, wave, denom):
    transformed = [fft.rfftn(v, workers=1) for v in velocity]
    divergence = sum(k * v for k, v in zip(wave, transformed)) / denom
    return np.array([fft.irfftn(v - k * divergence, s=velocity.shape[1:], workers=1).real for k, v in zip(wave, transformed)], dtype=np.float32)


def advect(field, departure):
    return ndimage.map_coordinates(field, departure, order=1, mode="constant", cval=0.0, prefilter=False)


def simulate():
    rng = np.random.default_rng(SEED)
    grid = np.indices((N, N, N), dtype=np.float32)
    z, y, x = (grid + 0.5) * (2.0 / N)
    z -= 1
    x -= 1
    cell = 2.0 / N
    shape = (N,) * 3
    edge = smooth(0.03, 0.16, 1 - np.abs(x)) * smooth(0.03, 0.16, 1 - np.abs(z)) * smooth(0.03, 0.15, y) * smooth(0.03, 0.18, 2 - y)
    octaves = (0.55 * noise(rng, shape, 3.8) + 0.28 * noise(rng, shape, 1.7) + 0.17 * noise(rng, shape, 0.65))
    rad = np.sqrt(x*x + z*z + (y - 0.52)**2)
    # The finite source has a corrugated front, with smaller lobes nested in it.
    source = np.zeros(shape, np.float32)
    # Overlapping irregular lobes seed the cauliflower structure of an impulsive
    # blast. Each lobe is a 3D density source, not a runtime sphere or billboard.
    lobes = [(0, 0.46, 0, 0.30)]
    for k in range(9):
        angle = k * (2*np.pi/9) + 0.13
        lobes.append((0.30*np.cos(angle), 0.40 + 0.12*np.sin(k*2.4), 0.30*np.sin(angle), 0.22 + 0.025*np.sin(k*1.7)))
    lobes += [(-0.13, 0.76, 0.09, 0.24), (0.15,0.73,-0.12,0.23)]
    for cx, cy, cz, radius in lobes:
        distance = np.sqrt((x-cx)**2 + (y-cy)**2 + (z-cz)**2)
        lobe = 1-smooth(radius-0.045, radius+0.015, distance+0.030*octaves)
        source = np.maximum(source, lobe)
    source *= (0.78 + 0.22*smooth(-1,1,octaves))
    density = np.zeros(shape, np.float32)
    heat = np.zeros_like(density)
    velocity = np.zeros((3,) + shape, np.float32)
    # Seed a divergence-free eddy field rather than uncorrelated voxel jitter.
    potential = [noise(rng, shape, 4.0) for _ in range(3)]
    velocity[0] = (np.gradient(potential[2], cell, axis=1) - np.gradient(potential[1], cell, axis=2)) * 0.010
    velocity[1] = (np.gradient(potential[0], cell, axis=2) - np.gradient(potential[2], cell, axis=0)) * 0.010
    velocity[2] = (np.gradient(potential[1], cell, axis=0) - np.gradient(potential[0], cell, axis=1)) * 0.010
    velocity *= np.exp(-rad * rad / 0.28)[None]
    wave = [fft.fftfreq(N)[:, None, None], fft.fftfreq(N)[None, :, None], fft.rfftfreq(N)[None, None, :]]
    wave = [np.asarray(k, dtype=np.float32) for k in wave]
    denom = sum(k*k for k in wave)
    denom[0, 0, 0] = 1
    outputs = [np.zeros((SIZE,) * 3, dtype=np.float32)]
    out_times = np.linspace(0, DURATION, FRAMES)
    output_grid = (np.indices((SIZE,)*3, dtype=np.float32) + 0.5) * N / SIZE - 0.5
    now = 0.0
    # Six substeps per animation frame keep rapid early source transport smooth.
    dt = DURATION / ((FRAMES - 1) * 6)
    for step in range(1, (FRAMES - 1) * 6 + 1):
        now = step * dt
        departure = grid - velocity * (dt / cell)
        velocity = np.array([advect(v, departure) for v in velocity])
        heat = advect(heat, departure) * np.exp(-dt * 1.0)
        forward = advect(density, departure)
        reverse = advect(forward, grid + velocity * (dt / cell))
        density = np.maximum(0, np.minimum(forward + 0.5 * (density - reverse), advect(ndimage.maximum_filter(density, size=3), departure)))
        density *= np.exp(-dt * 0.20)
        pulse = (1 - smooth(0.12, 0.25, now))
        density = np.maximum(density, source * pulse * 0.92)
        heat = np.maximum(heat, source * pulse * 2.5)
        velocity[1] += dt * (1.15 * heat - 0.12 * density)
        # Confine rotating plumes after advection; stronger curls survive coarse grids.
        wz = np.gradient(velocity[2], cell, axis=1) - np.gradient(velocity[1], cell, axis=2)
        wy = np.gradient(velocity[0], cell, axis=2) - np.gradient(velocity[2], cell, axis=0)
        wx = np.gradient(velocity[1], cell, axis=0) - np.gradient(velocity[0], cell, axis=1)
        curl = np.array([wz, wy, wx])
        norm = np.sqrt(np.sum(curl*curl, axis=0))
        grad = np.array(np.gradient(norm, cell))
        grad /= np.maximum(np.sqrt(np.sum(grad*grad, axis=0)), 1e-6)[None]
        confine = np.cross(grad, curl, axisa=0, axisb=0, axisc=0)
        velocity += dt * 0.085 * confine
        velocity = project(velocity, wave, denom)
        # Artist-directed explosive expansion is a finite nonzero-divergence source.
        # Added after projection deliberately: incompressibility alone erases a blast.
        blast = np.exp(-now * 9) * np.exp(-rad*rad / 0.13)
        velocity += dt * 7 * np.array([z, (y - 0.52) * 0.35, x]) * blast[None]
        velocity *= edge[None]
        density *= edge
        heat *= edge
        if step % 6 == 0:
            baked = advect(density, output_grid)
            # Preserve a readable rounded silhouette while letting the trailing wisps go.
            baked *= (1 - smooth(1.7, DURATION, now))
            outputs.append(np.clip(baked, 0, 1))
            print(f"frame {len(outputs)-1:02d}/{FRAMES-1}: t={now:.3f} max={baked.max():.3f} mass={baked.sum():.1f}", flush=True)
    outputs[-1].fill(0)
    return np.array(outputs)


def illuminate(density):
    """Integrate attenuation toward one fixed local-space key light."""
    n = density.shape[0]
    grid = np.indices(density.shape, dtype=np.float32)
    optical = np.zeros_like(density)
    # Half-voxel steps retain internal lobe shading with a linear texture filter.
    for offset in np.arange(0.5, n * 1.8, 1.5):
        optical += advect(density, grid + LIGHT[:, None, None, None] * offset) * (1.5 / n)
    return np.exp(-EXTINCTION * optical)


def preview(volumes, output):
    from PIL import Image, ImageDraw
    output.mkdir(parents=True, exist_ok=True)
    frames = []
    # Orthographic front and side diagnostic projections use exactly the R/G data.
    for fi, volume in enumerate(volumes):
        density, light = volume[..., 0] / 255, volume[..., 1] / 255
        transmission = np.ones((SIZE, SIZE))
        rgb = np.zeros((SIZE, SIZE, 3))
        for iz in range(SIZE - 1, -1, -1):
            alpha = 1 - np.exp(-density[iz] * EXTINCTION / SIZE)
            color = np.array([1.0, 0.96, 0.91])[None, None] * (0.035 + 0.40*light[iz, :, :, None])
            rgb += transmission[..., None] * alpha[..., None] * color
            transmission *= 1 - alpha
        bg = np.array([0.12, 0.16, 0.21])
        rgb += transmission[..., None]*bg
        image = Image.fromarray(np.rint(np.clip(rgb[::-1] ** (1/2.2), 0, 1)*255).astype(np.uint8)).resize((384,384), Image.Resampling.BICUBIC)
        ImageDraw.Draw(image).text((10,10), f"{fi*DURATION/(FRAMES-1):.2f}s", fill="white")
        frames.append(image)
    selected = [0, 2, 5, 9, 14, 20, 26, 31]
    sheet = Image.new("RGB", (384*4, 384*2))
    for i, fi in enumerate(selected): sheet.paste(frames[fi], ((i%4)*384, (i//4)*384))
    sheet.save(output / "bake-contact.jpg", quality=92)
    frames[0].save(output / "bake-preview.gif", save_all=True, append_images=frames[1:], duration=round(1000*DURATION/(FRAMES-1)), loop=0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path(__file__).resolve().parents[1]/"assets/fx")
    parser.add_argument("--preview", type=Path)
    args = parser.parse_args()
    density = simulate()
    volumes = np.empty((FRAMES, SIZE, SIZE, SIZE, 2), np.uint8)
    volumes[...,0] = np.rint(density*255).astype(np.uint8)
    for fi in range(FRAMES):
        volumes[fi,...,1] = np.rint(illuminate(density[fi])*255).astype(np.uint8)
    assert volumes.shape == (FRAMES, SIZE, SIZE, SIZE, 2)
    assert not volumes[0,...,0].any() and not volumes[-1,...,0].any(), "animation endpoints must be empty"
    edge_max = max(volumes[:,0,:,:,0].max(), volumes[:,-1,:,:,0].max(), volumes[:,:,0,:,0].max(), volumes[:,:,-1,:,0].max(), volumes[:,:,:,0,0].max(), volumes[:,:,:,-1,0].max())
    assert edge_max == 0, "density reaches texture boundary; enlarge simulation bounds before export"
    assert all(np.any(volumes[i,...,0] != volumes[i+1,...,0]) for i in range(FRAMES-1)), "frozen or repeated frame"
    args.out.mkdir(parents=True, exist_ok=True)
    data = volumes.tobytes(order="C")
    (args.out/"explosion-smoke.rg8.tmp").write_bytes(data)
    (args.out/"explosion-smoke.rg8.tmp").replace(args.out/"explosion-smoke.rg8")
    meta = {"format":"RG8_UNORM", "dimensions":[SIZE]*3, "frames":FRAMES, "duration_seconds":DURATION,
      "layout":"frame,z,y,x,channel; x fastest; frames stacked in texture Z", "bounds":{"min":[-1,0,-1],"max":[1,2,1]},
      "channels":{"R":"density", "G":"fixed local light transmittance"}, "light_direction_xyz":[-0.5,1.0,0.4],
      "extinction_per_texture_unit":EXTINCTION, "seed":SEED, "solver_dimensions":[N]*3,
      "bytes":len(data), "sha256":hashlib.sha256(data).hexdigest(),
      "density_max_per_frame":[float(v.max())/255 for v in volumes[...,0]],
      "density_mass_per_frame":[float(v.sum())/255 for v in volumes[...,0]],
      "numpy_version":np.__version__, "scipy_version":scipy.__version__, "source":"Original Warp procedural bake; no third-party smoke asset", "license":"MIT"}
    (args.out/"explosion-smoke.json").write_text(json.dumps(meta,indent=2)+"\n", encoding="utf-8")
    if args.preview: preview(volumes, args.preview)
    print(json.dumps({"bytes":len(data), "sha256":meta["sha256"]}), flush=True)

if __name__ == "__main__": main()
