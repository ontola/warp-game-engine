-- Reload rattle, after sfx.gd _synth_reload / _reload_cluster / _reload_tap (1603-1671).
-- Two clusters of 6-10 band-passed noise taps (25 ms each): mag out over the
-- first 12% of the reload (pitch falling 2.2 -> 0.4), mag in and rack over
-- 88-98% (rising 0.4 -> 2.0). Silence between. tanh over the sum.
--
-- Params:
--   duration  reload time in s (weapon.get_reload_time(), base 1.2); Godot snaps
--             clamp(duration, 0.2, 6) to 0.05 s (the cache bucket). Pass the real time.
-- Game: -14 dB, SPL 70, 2D for the reloading player, else room reverb at the gun, unit 12 m.
--   Stop the voice when the reload ends (or is cancelled), as Godot stops its player.
--   The voice ends at `duration` (Godot's buffer is that long).
--
-- Godot gives every tap its own noise and band-pass. Here each cluster is one
-- noise through one band-pass whose band jumps to each tap's pitch at its onset,
-- under the power sum of the taps' envelopes. Taps decay to 2% in 14 ms, so the
-- newest tap dominates, and the power sum matches overlapping independent taps.
-- (A noise and filter pair per tap, 20 slots, matches no closer and costs ~1.5x.)
length(param("duration", 1.2))
-- Keeps the voice alive through the silent gap between the clusters.
local _keepalive = env{ { 6.05, 0 } }

-- white44(): Godot's noise density (one uniform sample per 44.1 kHz tick).
-- godot_lp(x, k) / godot_hp(x, k): Godot's one-pole lerp(y, x, k) at 44.1 kHz, matched at
-- any rate, including its response near Nyquist (built in; see the Luau prelude).

local D = expr("clamp(x, 0.2, 6)", { x = param("duration", 1.2) })

local function cluster(f_start, f_end, p_begin, p_end)
  local t0 = D * f_start
  local span = expr("max(d * " .. (f_end - f_start) .. ", 0.02)", { d = D })
  local count = expr("floor(r)", { r = rand(6, 11) }) -- randi_range(6, 10)
  local taps = {}
  local prev_c = 1700 * p_begin
  for k = 0, 9 do
    local frac = expr("clamp((" .. k .. " + j) / n, 0, 1)", { j = rand(-0.15, 0.15), n = count })
    -- Godot starts a tap on the sample int(t_tap * 44100).
    local at = expr("floor((a + s * f) * 44100) / 44100", { a = t0, s = span, f = frac })
    local c = expr("1700 * lerp(" .. p_begin .. ", " .. p_end .. ", f) * j", { f = frac, j = rand(0.95, 1.05) })
    -- Slots past this voice's tap count stay off (their onset moves past the end).
    taps[#taps + 1] = { a = expr("a + 100 * (1 - (n > " .. k .. "))", { a = at, n = count }), g = rand(0.3, 0.5), dc = c - prev_c }
    prev_c = c
  end
  -- Two taps per formula (expr takes six inputs): the power sum of the tap
  -- envelopes, and the band centre, which steps to each tap's pitch at its onset.
  local power, centre = 0, 0
  for k = 1, #taps, 2 do
    local u, v = taps[k], taps[k + 1]
    power = expr([[p + step(a, t) * (t < a + 0.025) * g * g * exp(-max(t - a, 0) * 560)
      + step(b, t) * (t < b + 0.025) * h * h * exp(-max(t - b, 0) * 560)]], { p = power, a = u.a, g = u.g, b = v.a, h = v.g })
    centre = expr("x + step(a, t) * c + step(b, t) * d", { x = centre, a = u.a, c = u.dc, b = v.a, d = v.dc })
  end
  centre = centre + 1700 * p_begin
  -- Godot's k = 1 - exp(-2π fc / SR) at 44.1 kHz.
  local x = white44()
  local bp = godot_lp(x, expr("1 - exp(-tau * 1.5 * c / 44100)", { c = centre }))
    - godot_lp(x, expr("1 - exp(-tau * max(40, 0.6 * c) / 44100)", { c = centre }))
  return bp * expr("sqrt(p)", { p = power })
end

local s = cluster(0.00, 0.12, 2.2, 0.4) + cluster(0.88, 0.98, 0.4, 2.0)
return expr("tanh(x) * (t < d)", { x = s, d = D })
