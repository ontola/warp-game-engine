// Shared XR timing and controls. No game dependencies; metre units, -Z forward.
export function deadzone(value, threshold = 0.18) {
  return Number.isFinite(value) && Math.abs(value) > threshold
    ? Math.sign(value) * Math.min(1, (Math.abs(value) - threshold) / (1 - threshold)) : 0;
}
export class SnapTurn {
  armed = true;
  update(axis) {
    if (Math.abs(axis) < 0.25) this.armed = true;
    if (!this.armed || Math.abs(axis) < 0.7) return 0;
    this.armed = false;
    return -Math.sign(axis) * Math.PI / 6;
  }
  reset() { this.armed = true; }
}
// Keep press edges until a simulation tick consumes them, including at 90/120Hz.
export class Edges {
  held = {};
  pending = new Set();
  poll(values) {
    for (const [key, value] of Object.entries(values)) {
      if (value && !this.held[key]) this.pending.add(key);
      this.held[key] = !!value;
    }
  }
  take(key) { const value = this.pending.has(key); this.pending.delete(key); return value; }
  clear() { this.held = {}; this.pending.clear(); }
}
export class FixedStep {
  last = null;
  accumulator = 0;
  dt = 1 / 60;
  advance(ms, tick) {
    if (this.last === null) { this.last = ms; return 0; }
    this.accumulator += Math.min(0.066, Math.max(0, (ms - this.last) / 1000));
    this.last = ms;
    while (this.accumulator >= this.dt) { tick(); this.accumulator -= this.dt; }
    return this.accumulator / this.dt;
  }
  reset() { this.last = null; this.accumulator = 0; }
}
