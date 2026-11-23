-- Run this story again after changing a value to try the new build in place.
local weapon = "shotgun" -- rifle, shotgun, uzi or sniper
local params = {pellets=4, magazine=8, spread=.05}

return {
    id="loadout", title="Custom loadout", cast={},
    steps={{op="equip", weapon=weapon, params=params, text="Collect your custom loadout."}},
}
