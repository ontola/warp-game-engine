//! The spell rules against the Godot game's tests/spell_test.gd and
//! tests/balance_test.gd, with the counts and tables they print.

use std::collections::{BTreeMap, HashSet};

use spellstorm::el::{self, El, El::*};
use spellstorm::grammar::{self, FORMS, Form, Shape};
use spellstorm::queue::Queue;
use spellstorm::relics::{self, Mods};
use spellstorm::{balance as b, compose};

fn queue_of(presses: &[El]) -> Vec<El> {
    let mut q = Queue::new();
    for &e in presses {
        q.add(e);
    }
    q.slots
}

fn spell(presses: &[El], form: Form) -> grammar::Spell {
    compose(&queue_of(presses), form, &Mods::default())
}

/// Every reachable queue, breadth first over button presses, in the order
/// the Godot test finds them.
fn all_queues() -> Vec<Vec<El>> {
    let mut seen: Vec<Vec<El>> = vec![vec![]];
    let mut set: HashSet<Vec<El>> = seen.iter().cloned().collect();
    let mut frontier = vec![vec![]];
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for slots in &frontier {
            for e in el::BASE {
                let mut q = Queue::with(slots);
                q.add(e);
                if set.insert(q.slots.clone()) {
                    seen.push(q.slots.clone());
                    next.push(q.slots);
                }
            }
        }
        frontier = next;
    }
    seen
}

#[test]
fn queue_rules() {
    assert_eq!(queue_of(&[Water, Fire]), [Steam]);
    assert_eq!(queue_of(&[Fire, Water]), [Steam]);
    assert_eq!(queue_of(&[Water, Frost]), [Ice]);
    assert_eq!(queue_of(&[Earth, Fire]), [Magma]);
    assert_eq!(queue_of(&[Water, Earth]), [Mud]);
    assert_eq!(queue_of(&[Air, Lightning]), [Storm]);
    assert_eq!(queue_of(&[Fire, Frost]), [Fire, Frost]);
    assert_eq!(queue_of(&[Water, Fire, Frost]), [Steam, Frost]);
    assert_eq!(queue_of(&[Steam, Fire]), [Steam, Fire]);
    assert_eq!(queue_of(&[Fire, Fire, Fire, Fire]).len(), 3);
    assert_eq!(queue_of(&[Water, Fire, Water, Fire]), [Steam, Steam]);
}

#[test]
fn shapes_and_names() {
    use Form::*;
    assert_eq!(spell(&[Fire], Cast).name, "Flamethrower");
    assert_eq!(spell(&[Fire], Cast).shape, Shape::Spray);
    assert_eq!(spell(&[Earth, Fire, Fire], Cast).primary, Some(Magma));
    assert_eq!(spell(&[Earth, Poison], Cast).name, "Venomous Boulder");
    assert_eq!(spell(&[Lightning, Fire], Cast).shape, Shape::Strike);
    assert_eq!(spell(&[Lightning, Fire], Cast).name, "Burning Thunderbolt");
    assert_eq!(spell(&[Void, Fire], Cast).shape, Shape::Beam);
    assert_eq!(spell(&[Water, Frost, Ice], Cast).shape, Shape::Projectile);
    assert_eq!(spell(&[Fire, Fire, Fire], Cast).name, "Meteor");
    assert_eq!(spell(&[Fire, Fire, Fire], Nova).name, "Fire Nova");
    assert_eq!(spell(&[Water], Plant).name, "Puddle Trap");
    assert_eq!(spell(&[Earth, Earth], Plant).shape, Shape::Trap);
    assert_eq!(spell(&[Earth, Earth, Earth], Plant).shape, Shape::Sentry);
    assert_eq!(spell(&[Earth, Earth, Earth], Plant).name, "Stone Sentry");
    assert_eq!(spell(&[Poison], Enchant).name, "Poison Enchantment");
    assert_eq!(spell(&[Fire, Fire], Enchant).name, "Fire Enchantment");
    assert_eq!(spell(&[], Cast).shape, Shape::Staff);
}

#[test]
fn every_reachable_spell() {
    let queues = all_queues();
    let mut keys = HashSet::new();
    let mut names = HashSet::new();
    let mut shapes: BTreeMap<&str, usize> = BTreeMap::new();
    for slots in &queues {
        for form in FORMS {
            let s = compose(slots, form, &Mods::default());
            assert!(!s.name.is_empty());
            keys.insert(s.key.clone());
            names.insert(s.name.clone());
            *shapes.entry(s.shape.name()).or_default() += 1;
        }
    }
    // The Godot test prints: queues: 1541, distinct spells: 2414, distinct names: 2280
    assert_eq!(queues.len(), 1541);
    assert_eq!(keys.len(), 2414);
    assert_eq!(names.len(), 2280);
    let expect = [
        ("beam", 26),
        ("enchant", 1540),
        ("fizzle", 1),
        ("nova", 1540),
        ("overload", 1385),
        ("projectile", 76),
        ("sentry", 1385),
        ("spray", 23),
        ("staff", 2),
        ("stomp", 1),
        ("strike", 30),
        ("trap", 155),
    ];
    assert_eq!(shapes.into_iter().collect::<Vec<_>>(), expect);
    let castable = keys
        .iter()
        .filter(|k| !(k.ends_with("|staff") || k.ends_with("|stomp") || k.ends_with("|fizzle")))
        .count();
    assert_eq!(castable, grammar::TOTAL);
}

#[test]
fn tomes() {
    let mut tomed = [0; 13];
    for r in relics::ALL {
        assert!(r.text.len() <= 52, "{}", r.name);
        for m in r.mods {
            if let relics::Mod::El(e, _) = m {
                tomed[e.idx()] += 1;
            }
        }
    }
    assert_eq!(tomed, [1; 13]);
    let far = Mods {
        reach: 1.35,
        ..Mods::default()
    };
    assert!(compose(&[Fire], Form::Cast, &far).n.range > spell(&[Fire], Form::Cast).n.range);
    assert!(compose(&[Void], Form::Cast, &far).n.range > spell(&[Void], Form::Cast).n.range);
}

#[test]
fn balance_band() {
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    for slots in all_queues() {
        for form in FORMS {
            let s = compose(&slots, form, &Mods::default());
            if !b::measured(&s) {
                continue;
            }
            let mut sorted = slots.clone();
            sorted.sort();
            if !seen.insert((s.key.clone(), sorted)) {
                continue;
            }
            let r = b::ratio(&s) / b::target(&s);
            let w = b::worth(&s);
            rows.push((s, r, w));
        }
    }
    // The Godot test: "balance: 1538 spells, 0 out of band".
    assert_eq!(rows.len(), 1538);
    for (s, r, _) in &rows {
        assert!(
            *r >= b::BAND.0 && *r <= b::BAND.1,
            "{} {} {r}",
            s.name,
            s.shape.name()
        );
    }
    // Poison deals the most real damage per mana.
    let mut best = (None, 0.0);
    for e in el::BASE {
        let led: Vec<_> = rows.iter().filter(|row| row.0.primary == Some(e)).collect();
        if led.is_empty() {
            continue;
        }
        let avg = led
            .iter()
            .map(|(_, r, w)| w.real / w.total.max(0.001) * r)
            .sum::<f64>()
            / led.len() as f64;
        if avg > best.1 {
            best = (Some(e), avg);
        }
    }
    assert_eq!(best.0, Some(Poison));
    for e in el::ALL {
        let dps = b::bolt_dps(e);
        let goal = b::BOLT_DPS * b::skill(e);
        assert!(
            dps >= goal * b::BAND.0 && dps <= goal * b::BAND.1,
            "{}",
            e.name()
        );
    }
    // A few bolt numbers the Godot table prints.
    assert_eq!(b::ammo(Earth), 3);
    assert_eq!(b::ammo(Water), 20);
    assert_eq!(b::ammo(Poison), 7);
    assert!((b::skill(Poison) - 1.38).abs() < 0.005);
    assert!((b::bolt_dps(Poison) / b::flight(Some(Poison)).rate - 5.52).abs() < 0.01);
}

#[test]
fn enchantments() {
    for slots in [
        vec![Fire],
        vec![Fire, Fire],
        vec![Fire, Fire, Fire],
        vec![Frost, Frost],
        vec![Lightning, Lightning, Lightning],
        vec![Earth, Earth],
        vec![Water, Fire, Lightning],
        vec![Void, Void, Poison],
        vec![Lightning, Water, Poison],
    ] {
        let s = spell(&slots, Form::Enchant);
        let scales = b::bolt_scales(&s.counts, s.primary.unwrap(), b::ENCHANT_BOLT);
        let store = b::enchant_store(&s);
        let shot = b::enchant_shot(&s, &scales);
        let secs = store / (shot * b::flight(s.primary).rate);
        let swings = store / b::enchant_swing(&s);
        assert!(
            secs >= 2.0 && secs <= b::ENCHANT_TIME && swings >= 1.5,
            "{}",
            s.name
        );
    }
    // The Godot table: Fire 162, Fire Fire 356, Fire Fire Fire 583.
    let store = |s: &[El]| b::enchant_store(&spell(s, Form::Enchant)).round();
    assert_eq!(store(&[Fire]), 162.0);
    assert_eq!(store(&[Fire, Fire]), 356.0);
    assert_eq!(store(&[Fire, Fire, Fire]), 583.0);
}
