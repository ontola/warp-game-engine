//! The eight elements, the five fusions, and what each one does when it
//! lands (element.gd). Pure data, so tests and the grammar use it headless.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum El {
    Poison,
    Void,
    Fire,
    Frost,
    Water,
    Lightning,
    Earth,
    Air,
    Steam,
    Ice,
    Mud,
    Magma,
    Storm,
}

use El::*;

pub const BASE: [El; 8] = [Poison, Void, Fire, Frost, Water, Lightning, Earth, Air];
pub const FUSED: [El; 5] = [Steam, Ice, Mud, Magma, Storm];
pub const ALL: [El; 13] = [
    Poison, Void, Fire, Frost, Water, Lightning, Earth, Air, Steam, Ice, Mud, Magma, Storm,
];

pub const NAMES: [&str; 13] = [
    "Poison",
    "Void",
    "Fire",
    "Frost",
    "Water",
    "Lightning",
    "Earth",
    "Air",
    "Steam",
    "Ice",
    "Mud",
    "Magma",
    "Storm",
];

pub const COLOR: [[f32; 3]; 13] = [
    [0.55, 0.95, 0.12],
    [0.7, 0.2, 0.95],
    [1.0, 0.45, 0.1],
    [0.4, 0.88, 1.0],
    [0.2, 0.5, 1.0],
    [1.0, 0.88, 0.25],
    [0.62, 0.45, 0.28],
    [0.62, 1.0, 0.8],
    [0.85, 0.85, 0.9],
    [0.55, 0.85, 1.0],
    [0.42, 0.32, 0.2],
    [1.0, 0.3, 0.05],
    [0.55, 0.6, 0.95],
];

/// The face buttons: tap for the first, hold LB and tap for the second.
pub const PAIR_A: [El; 2] = [Poison, Void];
pub const PAIR_B: [El; 2] = [Fire, Frost];
pub const PAIR_X: [El; 2] = [Water, Air];
pub const PAIR_Y: [El; 2] = [Lightning, Earth];

/// Shape classes, heaviest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Solid,
    Energy,
    Spirit,
    Flow,
}

/// What one unit of an element does on a hit. Zero means absent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Effect {
    pub dmg: f64,
    pub poison: f64,
    pub push: f64,
    pub burn: f64,
    pub chill: f64,
    pub wet: f64,
    pub shock: f64,
    pub slow: f64,
}

const fn eff(dmg: f64) -> Effect {
    Effect {
        dmg,
        poison: 0.0,
        push: 0.0,
        burn: 0.0,
        chill: 0.0,
        wet: 0.0,
        shock: 0.0,
        slow: 0.0,
    }
}

pub const EFFECT: [Effect; 13] = [
    Effect {
        poison: 1.0,
        ..eff(6.0)
    },
    eff(16.0),
    Effect {
        burn: 1.0,
        ..eff(7.0)
    },
    Effect {
        chill: 1.0,
        ..eff(3.0)
    },
    Effect {
        wet: 1.0,
        push: 1.5,
        ..eff(1.0)
    },
    Effect {
        shock: 1.0,
        ..eff(12.0)
    },
    Effect {
        push: 2.0,
        ..eff(16.0)
    },
    Effect {
        push: 4.0,
        ..eff(1.0)
    },
    Effect {
        wet: 0.6,
        burn: 0.3,
        ..eff(10.0)
    },
    Effect {
        chill: 1.2,
        ..eff(12.0)
    },
    Effect {
        slow: 1.0,
        wet: 0.4,
        ..eff(5.0)
    },
    Effect {
        burn: 1.2,
        ..eff(14.0)
    },
    Effect {
        shock: 0.8,
        push: 2.0,
        ..eff(9.0)
    },
];

impl El {
    pub fn idx(self) -> usize {
        self as usize
    }

    pub fn from_index(i: usize) -> El {
        ALL[i]
    }

    pub fn name(self) -> &'static str {
        NAMES[self as usize]
    }

    pub fn color(self) -> [f32; 3] {
        COLOR[self as usize]
    }

    pub fn effect(self) -> &'static Effect {
        &EFFECT[self as usize]
    }

    pub fn kind(self) -> Kind {
        match self {
            Earth | Ice | Mud | Magma => Kind::Solid,
            Lightning | Storm => Kind::Energy,
            Poison | Void => Kind::Spirit,
            Fire | Frost | Water | Steam | Air => Kind::Flow,
        }
    }

    /// The two base elements a fusion is made of, in element.gd's order.
    pub fn fusion_parts(self) -> Option<[El; 2]> {
        Some(match self {
            Steam => [Water, Fire],
            Ice => [Water, Frost],
            Mud => [Water, Earth],
            Magma => [Earth, Fire],
            Storm => [Air, Lightning],
            _ => return None,
        })
    }

    pub fn is_fused(self) -> bool {
        self.fusion_parts().is_some()
    }

    /// Base elements this element is made of.
    pub fn parts(self) -> Vec<El> {
        match self.fusion_parts() {
            Some(p) => p.to_vec(),
            None => vec![self],
        }
    }

    pub fn contains(self, base: El) -> bool {
        self == base || self.fusion_parts().is_some_and(|p| p.contains(&base))
    }

    /// The fusion of two base elements, if they melt together.
    pub fn fuse(a: El, b: El) -> Option<El> {
        FUSED.into_iter().find(|f| {
            let p = f.fusion_parts().unwrap();
            (p[0] == a && p[1] == b) || (p[0] == b && p[1] == a)
        })
    }
}
