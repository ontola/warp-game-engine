-- The card pool, a port of Godot's card_library.gd (same ids, names,
-- texts, colours, rarities and effects). This file hot-reloads: edit and
-- save while the game runs. API: ../API.md (generated from the Rust side).
--
-- `apply(w)` mutates the player's Weapon like the Godot card does (field
-- names are weapon.gd's). `tags = { "firepower" }` marks cards that improve
-- the gun itself (every offer has one); `rarity = "rare"` makes a card less
-- likely. Godot helpers exist under the same names: Color, Vector3,
-- deg_to_rad, max, maxf, clampf, lerp.

local cards = {
	{ id = "dmg_up", name = "DAMAGE", desc = "", tags = { "firepower" }, color = Color(1.0, 0.25, 0.25),
		apply = function(w)
			w.damage_mult *= 1.5
		end },
	{ id = "healthy", name = "HEALTHY", desc = "", color = Color(0.45, 1.0, 0.55),
		apply = function(w)
			w.max_hp_bonus += 35
		end },
	{ id = "steady_hands", name = "STEADY HANDS", desc = "Tighter spread", tags = { "firepower" }, color = Color(0.65, 0.9, 1.0),
		apply = function(w)
			w.spread *= 0.65
		end },
	{ id = "precision", name = "PRECISION", desc = "Tighter spread and steadier aim", tags = { "firepower" }, color = Color(0.45, 0.95, 1.0),
		apply = function(w)
			w.spread *= 0.4
			w.recoil_per_shot *= 0.5
		end },
	{ id = "sneakers", name = "SNEAKERS", desc = "", color = Color(0.65, 1.0, 0.85),
		apply = function(w)
			w.move_speed_mult *= 1.15
		end },
	{ id = "rapid_fire", name = "RAPID FIRE", desc = "", tags = { "firepower" }, color = Color(1.0, 0.8, 0.2),
		apply = function(w)
			w.fire_rate_mult *= 1.6
		end },
	{ id = "bullet_speed", name = "FAST ROUNDS", desc = "Bullets travel faster", tags = { "firepower" }, color = Color(0.72, 0.94, 1.0),
		apply = function(w)
			w.bullet_speed_mult *= 1.5
			w.bullet_color = lerp(w.bullet_color, Color(0.65, 0.92, 1.0), 0.3)
		end },
	{ id = "big_mag", name = "BIG MAG", desc = "", tags = { "firepower" }, color = Color(0.3, 0.8, 1.0),
		apply = function(w)
			w.mag_size_bonus += 20
		end },
	{ id = "quick_reload", name = "QUICK RELOAD", desc = "", tags = { "firepower" }, color = Color(0.3, 1.0, 0.6),
		apply = function(w)
			w.reload_mult *= 2.0
		end },
	{ id = "ricochet", name = "RICOCHET", desc = "Bullets bounce off walls", tags = { "firepower" }, color = Color(1.0, 0.65, 0.25),
		apply = function(w)
			w.ricochet_count += 2
		end },
	{ id = "last_bullet", name = "LAST BULLET", desc = "Final round in mag deals 5× damage", tags = { "firepower" }, color = Color(1.0, 0.82, 0.25),
		apply = function(w)
			w.last_shot_damage_mult *= 5.0
			w.bullet_color = lerp(w.bullet_color, Color(1.0, 0.75, 0.15), 0.4)
		end },
	{ id = "silencer", name = "SILENCER", desc = "Hidden shots; faint tracer for you; −10% damage", color = Color(0.55, 0.58, 0.62),
		apply = function(w)
			w.silencer_stacks += 1
			w.damage_mult *= 0.9
			w.bullet_color = lerp(w.bullet_color, Color(0.45, 0.48, 0.52), 0.55)
		end },
	{ id = "shotgun", name = "SHOTGUN", desc = "Multi-projectile burst", tags = { "firepower" }, color = Color(1.0, 0.5, 0.75),
		apply = function(w)
			w.extra_projectiles += 2
			w.reload_mult *= 1.5
			w.damage_mult *= 1.5
			w.fire_rate_mult *= 0.3
			w.bullet_speed_mult *= 0.75
			w.spread = max(w.spread, deg_to_rad(2.0)) + deg_to_rad(2.0)
		end },
	{ id = "extra_barrel", name = "EXTRA BARREL", desc = "Each shot fires an extra parallel bullet", rarity = "rare", tags = { "firepower" }, color = Color(0.72, 0.74, 0.82),
		apply = function(w)
			w.extra_projectiles += 1
		end },
	{ id = "lifesteal", name = "LIFESTEAL", desc = "Damage enemies, gain some health", color = Color(0.8, 0.2, 0.85),
		apply = function(w)
			w.lifesteal += 0.4
			w.bullet_color = lerp(w.bullet_color, Color(0.85, 0.15, 0.8), 0.4)
		end },
	{ id = "explosive", name = "EXPLOSIVE ROUNDS", desc = "Bullets explode on impact (stackable)", tags = { "firepower" }, color = Color(1.0, 0.4, 0.1),
		apply = function(w)
			-- First stack establishes a sizable baseline; each extra one grows the blast.
			if w.explosive_radius <= 0 then w.explosive_radius += 4.0 else w.explosive_radius += 2.5 end
			w.explosive_damage += 25.0
			w.bullet_color = lerp(w.bullet_color, Color(1.0, 0.45, 0.08), 0.45)
		end },
	{ id = "headhunter", name = "HEADHUNTER", desc = "Deadly headshots", tags = { "firepower" }, color = Color(0.9, 0.9, 1.0),
		apply = function(w)
			w.headshot_mult *= 2.0
		end },
	{ id = "hitscan", name = "HITSCAN", desc = "Extreme muzzle velocity and near-perfect accuracy", rarity = "rare", tags = { "firepower" }, color = Color(0.7, 0.95, 1.0),
		apply = function(w)
			w.bullet_speed_mult *= 4.0
			w.spread *= 0.1
			w.bullet_color = lerp(w.bullet_color, Color(0.72, 0.96, 1.0), 0.55)
		end },
	{ id = "heavy_rounds", name = "HEAVY ROUNDS", desc = "Powerful but slow projectiles", rarity = "rare", tags = { "firepower" }, color = Color(0.7, 0.4, 0.2),
		apply = function(w)
			w.damage_mult *= 2.5
			w.bullet_speed_mult *= 0.65
			w.fire_rate_mult *= 0.7
			w.bullet_color = lerp(w.bullet_color, Color(1.0, 0.55, 0.2), 0.3)
		end },
	{ id = "sniper", name = "SNIPER", desc = "Railgun-like shots with RMB Zoom", rarity = "rare", tags = { "firepower" }, color = Color(0.55, 0.85, 1.0),
		apply = function(w)
			w.damage_mult *= 3.0
			w.bullet_speed_mult *= 2.5
			w.fire_rate_mult *= 0.4
			w.spread *= 0.1
			w.headshot_mult *= 1.5
			w.special = "zoom"
			w.bullet_color = lerp(w.bullet_color, Color(0.55, 0.85, 1.0), 0.6)
		end },
	{ id = "uzi", name = "UZI", desc = "High-volume automatic fire", rarity = "rare", tags = { "firepower" }, color = Color(1.0, 0.65, 0.25),
		apply = function(w)
			w.fire_rate_mult *= 3.0
			w.mag_size_bonus += 20
			w.damage_mult *= 0.7
			w.spread *= 2
			w.bullet_color = lerp(w.bullet_color, Color(1.0, 0.55, 0.15), 0.5)
		end },
	{ id = "minigun", name = "MINIGUN", desc = "Ten barrels, absurd fire rate, tiny bullet damage", rarity = "rare", tags = { "firepower" }, color = Color(1.0, 0.78, 0.28),
		apply = function(w)
			w.fire_rate_mult *= 10.0
			w.mag_size_bonus += 95
			w.damage_mult *= 0.2
			w.spread = max(w.spread, deg_to_rad(1.2)) + deg_to_rad(1.2)
			w.recoil_per_shot *= 0.65
			w.bullet_color = lerp(w.bullet_color, Color(1.0, 0.72, 0.12), 0.65)
		end },
	{ id = "ninja", name = "NINJA", desc = "Shift ends with a quick slash", color = Color(0.35, 0.38, 0.48),
		apply = function(w)
			w.dash_end_melee = true
			w.bullet_color = lerp(w.bullet_color, Color(0.45, 0.48, 0.62), 0.25)
		end },
	{ id = "homing", name = "HOMING", desc = "Bullets lazily curve toward enemies", rarity = "rare", tags = { "firepower" }, color = Color(0.7, 1.0, 0.55),
		apply = function(w)
			w.homing += 25.0
			w.bullet_speed_mult *= 0.6
			w.bullet_color = lerp(w.bullet_color, Color(0.6, 1.0, 0.45), 0.35)
		end },
	{ id = "haymaker", name = "HAYMAKER", desc = "Enormous bullets with massive kick", rarity = "rare", tags = { "firepower" }, color = Color(0.65, 0.85, 1.0),
		apply = function(w)
			w.knockback_mult *= 3.5
			w.damage_mult *= 1.5
			w.bullet_color = lerp(w.bullet_color, Color(0.6, 0.9, 1.0), 0.45)
		end },
	{ id = "big_head", name = "BIG HEAD", desc = "Easier to land headshots, but easier to be headshotted", tags = { "firepower" }, color = Color(0.95, 0.55, 0.9),
		apply = function(w)
			w.damage_mult *= 1.8
			w.head_scale *= 1.8
			w.bullet_color = lerp(w.bullet_color, Color(1.0, 0.5, 0.85), 0.3)
		end },
	{ id = "chonky", name = "CHONKY", desc = "Higher durability, but a larger target", color = Color(0.55, 0.9, 0.55),
		apply = function(w)
			w.max_hp_bonus += 75
			w.body_scale *= 1.5
		end },
	{ id = "slenderman", name = "SLENDERMAN", desc = "Tall and narrow — hard to hit body-on", color = Color(0.35, 0.35, 0.45),
		apply = function(w)
			w.body_scale_axes *= Vector3(0.72, 1.8, 0.72)
			warp(w, "Neck", Vector3(1.0, 1.5, 1.0))
			warp(w, "LeftArm", Vector3(0.85, 1.3, 0.85))
			warp(w, "RightArm", Vector3(0.85, 1.3, 0.85))
		end },
	{ id = "flatfish", name = "FLATFISH", desc = "Squished — small from the front, big from the sides", color = Color(0.55, 0.85, 1.0),
		apply = function(w)
			w.body_scale_axes *= Vector3(0.65, 1.0, 1.45)
		end },
	{ id = "acrobat", name = "ACROBAT", desc = "Jump like a ninja", color = Color(0.5, 0.9, 1.0),
		apply = function(w)
			w.extra_jumps += 2
		end },
	{ id = "glass_cannon", name = "GLASS CANNON", desc = "Huge damage and speed, much lower health", tags = { "firepower" }, color = Color(1.0, 0.25, 0.45),
		apply = function(w)
			w.damage_mult *= 2.2
			w.bullet_speed_mult *= 1.35
			w.max_hp_bonus -= 45
			w.bullet_color = lerp(w.bullet_color, Color(1.0, 0.12, 0.32), 0.45)
		end },
	{ id = "spray", name = "SPRAY", desc = "Full-auto bullet hose — held fire sprays wild", tags = { "firepower" }, color = Color(1.0, 0.78, 0.18),
		apply = function(w)
			w.fire_rate_mult *= 2.2
			w.mag_size_bonus += 15
			w.damage_mult *= 0.65
			w.spread += deg_to_rad(1.0)
			w.recoil_per_shot *= 3.0
			w.bullet_color = lerp(w.bullet_color, Color(1.0, 0.72, 0.12), 0.4)
		end },
	{ id = "careful_planning", name = "CAREFUL PLANNING", desc = "Massive shots, slow fire and reload", tags = { "firepower" }, color = Color(0.95, 0.65, 0.35),
		apply = function(w)
			w.damage_mult *= 2.5
			w.fire_rate_mult *= 0.45
			w.reload_mult *= 0.65
			w.recoil_per_shot *= 1.25
		end },
	{ id = "wind_up", name = "WIND UP", desc = "Harder, faster bullets with slower cadence", tags = { "firepower" }, color = Color(0.45, 0.85, 1.0),
		apply = function(w)
			w.damage_mult *= 1.7
			w.bullet_speed_mult *= 1.8
			w.fire_rate_mult *= 0.65
			w.bullet_color = lerp(w.bullet_color, Color(0.35, 0.8, 1.0), 0.4)
		end },
	{ id = "grow", name = "GROW", desc = "Bullets swell and gain damage the farther they fly", tags = { "firepower" }, color = Color(0.6, 1.0, 0.45),
		apply = function(w)
			w.grow_damage_per_meter += 0.015
			w.bullet_speed_mult *= 0.85
			w.bullet_color = lerp(w.bullet_color, Color(0.45, 1.0, 0.25), 0.4)
		end },
	{ id = "poison", name = "POISON", desc = "Hits deal more damage, spread over time", tags = { "firepower" }, color = Color(0.45, 1.0, 0.25),
		apply = function(w)
			w.damage_over_time += 0.7
			w.dot_duration = maxf(w.dot_duration, 3.5)
			w.bullet_color = lerp(w.bullet_color, Color(0.35, 1.0, 0.1), 0.45)
		end },
	{ id = "chilling_rounds", name = "CHILLING ROUNDS", desc = "Hits heavily slow enemy movement", tags = { "firepower" }, color = Color(0.55, 0.9, 1.0),
		apply = function(w)
			w.slow_on_hit = clampf(w.slow_on_hit * 0.48, 0.22, 1.0)
			w.slow_duration += 2.6
			w.bullet_speed_mult *= 0.9
			w.bullet_color = lerp(w.bullet_color, Color(0.45, 0.85, 1.0), 0.45)
		end },
	{ id = "scavenger", name = "SCAVENGER", desc = "Damaging enemies feeds ammo back into the mag", tags = { "firepower" }, color = Color(0.3, 1.0, 0.55),
		apply = function(w)
			w.reload_on_hit += 2
		end },
	{ id = "phoenix", name = "PHOENIX", desc = "Revive once per round at low health", rarity = "rare", color = Color(1.0, 0.45, 0.12),
		apply = function(w)
			w.phoenix_revives += 1
			w.max_hp_bonus -= 10
			w.bullet_color = lerp(w.bullet_color, Color(1.0, 0.45, 0.08), 0.35)
		end },
	{ id = "dodge", name = "DODGE", desc = "Shift grants brief invulnerability", color = Color(0.72, 0.92, 1.0),
		apply = function(w)
			w.dash_iframes = true
		end },
	{ id = "drill_rounds", name = "DRILL ROUNDS", desc = "Bullets bore through walls before stopping", tags = { "firepower" }, color = Color(0.75, 0.75, 0.82),
		apply = function(w)
			w.world_pierce_count += 2
			w.damage_mult *= 1.25
			w.bullet_speed_mult *= 0.8
		end },
	{ id = "landmine_rounds", name = "LANDMINE ROUNDS", desc = "Bullet impacts plant proximity mines", rarity = "rare", tags = { "firepower" }, color = Color(0.05, 0.9, 0.72),
		apply = function(w)
			w.impact_mines += 1
			w.fire_rate_mult *= 0.8
			w.bullet_speed_mult *= 0.85
			w.bullet_color = lerp(w.bullet_color, Color(0.0, 0.95, 0.75), 0.5)
		end },
	{ id = "echo", name = "ECHO", desc = "RMB repeats a moment later, and recharges faster", color = Color(0.8, 0.55, 1.0),
		apply = function(w)
			w.special_echo_count += 1
			w.special_cooldown_mult *= 0.85
		end },
	{ id = "tactical_reload", name = "TACTICAL RELOAD", desc = "RMB also loads rounds into your mag", tags = { "firepower" }, color = Color(0.3, 1.0, 0.75),
		apply = function(w)
			w.special_reload_amount += 3
		end },
	{ id = "refreshed", name = "REFRESHED", desc = "Damaging enemies refunds RMB cooldown", color = Color(0.65, 0.95, 1.0),
		apply = function(w)
			w.special_cooldown_refund_on_hit += 0.45
		end },
	{ id = "teleport", name = "TELEPORT", desc = "RMB: teleport where you aim", color = Color(0.75, 0.35, 1.0),
		apply = function(w)
			w.special = "teleport"
		end },
	{ id = "sword", name = "SWORD", desc = "RMB: powerful melee slash", color = Color(0.8, 0.8, 0.9),
		apply = function(w)
			w.special = "sword"
		end },
	{ id = "air_strike", name = "AIR STRIKE", desc = "RMB: call a rocket on your crosshair", rarity = "rare", color = Color(1.0, 0.28, 0.12),
		apply = function(w)
			w.special = "air_strike"
		end },
	{ id = "ion_cannon", name = "ION CANNON", desc = "RMB: orbital beam on your crosshair", rarity = "rare", color = Color(0.35, 0.72, 1.0),
		apply = function(w)
			w.special = "ion_cannon"
		end },
	{ id = "cluster_grenade", name = "CLUSTER", desc = "RMB: grenade that splits into smaller blasts", rarity = "rare", color = Color(1.0, 0.52, 0.12),
		apply = function(w)
			w.special = "cluster_grenade"
		end },
	{ id = "backblast", name = "BACKBLAST", desc = "Shift leaves a bomb where you dashed from", color = Color(1.0, 0.42, 0.12),
		apply = function(w)
			w.dash_spawn_bomb = true
		end },
	{ id = "bazooka", name = "BAZOOKA", desc = "Slow rocket-like shots with a massive explosion", rarity = "rare", tags = { "firepower" }, color = Color(1.0, 0.25, 0.0),
		apply = function(w)
			w.damage_mult *= 4.0
			w.fire_rate_mult *= 0.12
			w.reload_mult *= 0.45
			w.bullet_speed_mult *= 0.12
			w.explosive_radius += 12.0
			w.explosive_damage += 180.0
			w.bullet_drop *= 0.1  -- rocket thrust mostly cancels gravity
			w.bullet_color = lerp(w.bullet_color, Color(1.0, 0.1, 0.0), 0.8)
		end },
}

return { cards = cards }
