-- Reload rattle, after sfx.gd _synth_reload / _reload_cluster / _reload_tap (1603-1671).
-- Two clusters of 6-10 band-passed noise taps (25 ms each): mag out over the
-- first 12% of the reload (pitch falling 2.2 -> 0.4), mag in and rack over
-- 88-98% (rising 0.4 -> 2.0). Silence between. tanh over the sum.
--
-- Params:
--   duration  reload time in s (weapon.get_reload_time(), base 1.2); Godot snaps
--             clamp(duration, 0.2, 6) to 0.05 s.
-- Game: -14 dB, 2D for the reloading player, else room reverb at the gun, unit 12 m, SPL 70.
--   Stop the voice when the reload ends (or is cancelled). It is silent after
--   `duration`, but the synth cannot end a voice at a parameter time, so it is
--   kept alive (silent) up to 6.05 s unless stopped.
--
-- Each tap is its own noise and filter pair, gated on at its start time, so the
-- patch has 20 tap slots (10 per cluster; slots past this voice's tap count stay silent).
length(6.05)
local _keepalive = env{ { 6.05, 0 } }

-- Godot draws one uniform white sample per 44.1 kHz tick; at a higher rate the same
-- variance spreads over more bandwidth, so scale it to keep Godot's noise density.
local function white() return noise() * expr("sqrt(sr / 44100)") end

local D = expr("clamp(x, 0.2, 6)", { x = param("duration", 1.2) })
local taps = {}

local function cluster(f_start, f_end, p_begin, p_end)
  local t0 = D * f_start
  local span = expr("max(d * " .. (f_end - f_start) .. ", 0.02)", { d = D })
  local count = expr("floor(r)", { r = rand(6, 11) }) -- randi_range(6, 10)
  for k = 0, 9 do
    local frac = expr("clamp((" .. k .. " + j) / n, 0, 1)", { j = rand(-0.15, 0.15), n = count })
    local at = expr("floor((a + s * f) * 44100) / 44100", { a = t0, s = span, f = frac })
    local c = expr("1700 * lerp(" .. p_begin .. ", " .. p_end .. ", f) * j", { f = frac, j = rand(0.95, 1.05) })
    local x = white() * expr("step(a, t) * (n > " .. k .. ")", { a = at, n = count })
    -- Godot's k = 1 - exp(-2π fc / SR) is exact, so these cutoffs are in Hz as written.
    local bp = lowpass1(x, c * 1.5) - lowpass1(x, expr("max(40, 0.6 * c)", { c = c }))
    taps[#taps + 1] = bp * expr("exp(-(t - a) * 280) * (t < a + 0.025) * g", { a = at, g = rand(0.3, 0.5) })
  end
end

cluster(0.00, 0.12, 2.2, 0.4)
cluster(0.88, 0.98, 0.4, 2.0)

return expr("tanh(x) * (t < d)", { x = mix(table.unpack(taps)), d = D })
