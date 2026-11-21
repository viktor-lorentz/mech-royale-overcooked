//! Typed view of the hello `config`.
use serde_json::Value;
use std::collections::HashMap;

pub const GUN: usize = 0;
pub const MORTAR: usize = 1;
pub const LASER: usize = 2;
pub const LIGHTNING: usize = 3;
pub const KIND_NAMES: [&str; 4] = ["gun", "mortar", "laser", "lightning"];

pub fn kind_idx(s: &str) -> Option<usize> {
    KIND_NAMES.iter().position(|k| *k == s)
}

pub fn num(v: &Value, path: &[&str], d: f64) -> f64 {
    let mut c = v;
    for p in path {
        c = &c[*p];
    }
    c.as_f64().unwrap_or(d)
}

#[derive(Clone, Debug)]
pub struct Effect {
    pub path: String,
    pub mul: Option<f64>,
    pub add: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct UpgradeDef {
    pub id: String,
    pub requires: Option<String>,
    pub max: u32,
    pub effects: Vec<Effect>,
}

#[derive(Clone, Debug)]
pub struct ZonePhase {
    pub hold_s: f64,
    pub shrink_s: f64,
    pub radius: f64,
    pub dps: f64,
}

#[derive(Clone)]
pub struct Cfg {
    pub raw: Value,
    pub dt: f64,
    pub decide_every: u64,
    pub duration: f64,
    pub n_mechs: usize,
    pub spawn_sep: f64,
    pub spawn_frac: f64,
    pub margin: f64,
    pub mech: HashMap<String, f64>,
    pub weapons: [HashMap<String, f64>; 4],
    pub vis_range: f64,
    pub vis_fov: f64,
    pub vis_near: f64,
    pub memory_s: f64,
    pub bearing_noise: f64,
    pub snd_near: f64,
    pub snd_mid: f64,
    pub step_speed: f64,
    pub step_every: f64,
    pub snd_range: HashMap<String, f64>,
    pub gun_radius: f64,
    pub npc_radius: f64,
    pub npc_height: f64,
    pub npc_hp: f64,
    pub npc_count: usize,
    pub npc_speed: f64,
    pub npc_leash: f64,
    pub npc_chase_leash: f64,
    pub npc_aggro: f64,
    pub npc_deaggro: f64,
    pub npc_damage: f64,
    pub npc_cd: f64,
    pub npc_proj_speed: f64,
    pub npc_spread: f64,
    pub npc_cubes: u32,
    pub cube_pickup_r: f64,
    pub cube_hp: f64,
    pub cube_dmg: f64,
    pub cube_cap: f64,
    pub cube_extra: u32,
    pub cube_size: f64,
    pub thresholds: Vec<f64>,
    pub crate_count: usize,
    pub crate_roof: f64,
    pub crate_hp: f64,
    pub crate_r: f64,
    pub crate_h: f64,
    pub crate_cubes: u32,
    pub pickup_r: f64,
    pub equip_s: f64,
    pub floor_count: usize,
    pub floor_roof: f64,
    pub crate_weapon_chance: f64,
    pub loot_weights: [f64; 4],
    pub drop_times: Vec<f64>,
    pub drop_fall: f64,
    pub drop_radius: f64,
    pub drop_capture: f64,
    pub drop_cubes: u32,
    pub drop_spread: f64,
    pub zone_center: [f64; 2],
    pub zone_radius: f64,
    pub phases: Vec<ZonePhase>,
    pub offer_timeout: f64,
    pub upgrades: Vec<UpgradeDef>,
}

fn map_of(v: &Value) -> HashMap<String, f64> {
    let mut m = HashMap::new();
    if let Some(o) = v.as_object() {
        for (k, x) in o {
            if let Some(f) = x.as_f64() {
                m.insert(k.clone(), f);
            }
        }
    }
    m
}

impl Cfg {
    pub fn from_hello(h: &Value) -> Cfg {
        let c = &h["config"];
        let n = |p: &[&str], d: f64| num(c, p, d);
        let hz = n(&["match", "tick_hz"], 20.0);
        let w = &c["weapons"];
        let weapons = [map_of(&w["gun"]), map_of(&w["mortar"]), map_of(&w["laser"]), map_of(&w["lightning"])];
        let mut upgrades = vec![];
        for u in c["upgrades"].as_array().cloned().unwrap_or_default() {
            let effects = u["effects"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|e| Effect {
                    path: e["path"].as_str().unwrap_or("").to_string(),
                    mul: e["mul"].as_f64(),
                    add: e["add"].as_f64(),
                })
                .collect();
            upgrades.push(UpgradeDef {
                id: u["id"].as_str().unwrap_or("").to_string(),
                requires: u["requires"].as_str().map(|s| s.to_string()),
                max: u["max"].as_u64().unwrap_or(1) as u32,
                effects,
            });
        }
        let phases = c["zone"]["phases"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|p| ZonePhase {
                hold_s: p["hold_s"].as_f64().unwrap_or(30.0),
                shrink_s: p["shrink_s"].as_f64().unwrap_or(30.0),
                radius: p["radius"].as_f64().unwrap_or(0.0),
                dps: p["dps"].as_f64().unwrap_or(20.0),
            })
            .collect();
        let arr = |p: &[&str]| -> Vec<f64> {
            let mut x = c;
            for k in p {
                x = &x[*k];
            }
            x.as_array().map(|a| a.iter().filter_map(|v| v.as_f64()).collect()).unwrap_or_default()
        };
        let zc = arr(&["zone", "start_center"]);
        Cfg {
            raw: h.clone(),
            dt: 1.0 / hz,
            decide_every: n(&["match", "decide_every"], 2.0) as u64,
            duration: n(&["match", "duration_s"], 480.0),
            n_mechs: n(&["match", "mechs"], 16.0) as usize,
            spawn_sep: n(&["match", "spawn_min_separation"], 130.0),
            spawn_frac: n(&["match", "spawn_zone_fraction"], 0.8),
            margin: n(&["match", "arena_margin"], 30.0),
            mech: map_of(&c["mech"]),
            weapons,
            vis_range: n(&["vision", "range"], 200.0),
            vis_fov: n(&["vision", "fov_deg"], 120.0).to_radians(),
            vis_near: n(&["vision", "near_range"], 45.0),
            memory_s: n(&["vision", "memory_s"], 5.0),
            bearing_noise: n(&["sound", "bearing_noise_deg"], 7.0).to_radians(),
            snd_near: n(&["sound", "near"], 60.0),
            snd_mid: n(&["sound", "mid"], 160.0),
            step_speed: n(&["sound", "step_speed"], 5.0),
            step_every: n(&["sound", "step_every"], 0.6),
            snd_range: map_of(&c["sound"]["range"]),
            gun_radius: n(&["projectile", "gun_radius"], 0.3),
            npc_radius: n(&["npc", "radius"], 1.8),
            npc_height: n(&["npc", "height"], 3.5),
            npc_hp: n(&["npc", "hp"], 350.0),
            npc_count: n(&["npc", "count"], 12.0) as usize,
            npc_speed: n(&["npc", "speed"], 5.0),
            npc_leash: n(&["npc", "leash"], 30.0),
            npc_chase_leash: n(&["npc", "chase_leash"], 60.0),
            npc_aggro: n(&["npc", "aggro_range"], 45.0),
            npc_deaggro: n(&["npc", "deaggro_range"], 90.0),
            npc_damage: n(&["npc", "damage"], 25.0),
            npc_cd: n(&["npc", "cooldown"], 1.0),
            npc_proj_speed: n(&["npc", "projectile_speed"], 90.0),
            npc_spread: n(&["npc", "spread_deg"], 2.5).to_radians(),
            npc_cubes: n(&["npc", "cubes"], 2.0) as u32,
            cube_pickup_r: n(&["cubes", "pickup_radius"], 3.5),
            cube_hp: n(&["cubes", "hp_per_cube"], 0.04),
            cube_dmg: n(&["cubes", "damage_per_cube"], 0.04),
            cube_cap: n(&["cubes", "stat_cap"], 10.0),
            cube_extra: n(&["cubes", "death_drop_extra"], 2.0) as u32,
            cube_size: n(&["cubes", "size_per_cube"], 0.05),
            thresholds: arr(&["cubes", "upgrade_thresholds"]),
            crate_count: n(&["crates", "count"], 36.0) as usize,
            crate_roof: n(&["crates", "roof_fraction"], 0.25),
            crate_hp: n(&["crates", "hp"], 300.0),
            crate_r: n(&["crates", "radius"], 1.4),
            crate_h: n(&["crates", "height"], 2.4),
            crate_cubes: n(&["crates", "cubes"], 1.0) as u32,
            pickup_r: n(&["loot", "pickup_radius"], 4.0),
            equip_s: n(&["loot", "equip_s"], 1.0),
            floor_count: n(&["loot", "floor_count"], 40.0) as usize,
            floor_roof: n(&["loot", "floor_roof_fraction"], 0.25),
            crate_weapon_chance: n(&["loot", "crate_weapon_chance"], 0.5),
            loot_weights: [
                n(&["loot", "weights", "gun"], 0.0),
                n(&["loot", "weights", "mortar"], 1.0),
                n(&["loot", "weights", "laser"], 1.0),
                n(&["loot", "weights", "lightning"], 1.0),
            ],
            drop_times: arr(&["drops", "times"]),
            drop_fall: n(&["drops", "fall_s"], 25.0),
            drop_radius: n(&["drops", "radius"], 8.0),
            drop_capture: n(&["drops", "capture_s"], 4.0),
            drop_cubes: n(&["drops", "cubes"], 3.0) as u32,
            drop_spread: n(&["drops", "spread"], 0.7),
            zone_center: [zc.get(0).copied().unwrap_or(41.3), zc.get(1).copied().unwrap_or(-72.8)],
            zone_radius: n(&["zone", "start_radius"], 560.0),
            phases,
            offer_timeout: n(&["bot", "offer_timeout_s"], 5.0),
            upgrades,
        }
    }
}

/// Hot mech numbers (already scaled by cube size).
#[derive(Clone, Default, Debug)]
pub struct Ms {
    pub size: f64,
    pub radius: f64,
    pub height: f64,
    pub eye: f64,
    pub muzzle: f64,
    pub walk: f64,
    pub boost: f64,
    pub ground_accel: f64,
    pub air_accel: f64,
    pub gravity: f64,
    pub jump: f64,
    pub jet_accel: f64,
    pub max_rise: f64,
    pub step_up: f64,
    pub max_slope: f64,
    pub energy_regen: f64,
    pub en_delay: f64,
    pub boost_drain: f64,
    pub jet_drain: f64,
    pub turn_rate: f64,
    pub pitch_rate: f64,
    pub min_pitch: f64,
    pub max_pitch: f64,
    pub regen_delay: f64,
    pub regen_per_s: f64,
    pub charge_move: f64,
    pub self_dmg: f64,
    pub armor: f64,
    pub rear_mult: f64,
    pub front_arc: f64,
    pub near_bonus: f64,
    pub vision_bonus: f64,
    pub hearing: f64,
}

impl Ms {
    pub fn from_map(m: &HashMap<String, f64>, cubes: u32, size_per_cube: f64) -> Ms {
        let g = |k: &str, d: f64| m.get(k).copied().unwrap_or(d);
        let size = 1.0 + size_per_cube * cubes as f64;
        Ms {
            size,
            radius: g("radius", 2.5) * size,
            height: g("height", 9.0) * size,
            eye: g("eye_height", 8.0) * size,
            muzzle: g("muzzle_height", 7.0) * size,
            walk: g("walk_speed", 11.7),
            boost: g("boost_speed", 19.5),
            ground_accel: g("ground_accel", 58.5),
            air_accel: g("air_accel", 12.0),
            gravity: g("gravity", 22.0),
            jump: g("jump_speed", 11.0),
            jet_accel: g("jet_accel", 34.0),
            max_rise: g("max_rise_speed", 10.0),
            step_up: g("step_up", 1.2),
            max_slope: g("max_slope_deg", 50.0).to_radians(),
            energy_regen: g("energy_regen", 18.0),
            en_delay: g("energy_regen_delay", 0.6),
            boost_drain: g("boost_drain", 17.0),
            jet_drain: g("jet_drain", 15.0),
            turn_rate: g("turn_rate", 4.0),
            pitch_rate: g("pitch_rate", 3.0),
            min_pitch: g("min_pitch", -0.7),
            max_pitch: g("max_pitch", 1.3),
            regen_delay: g("regen_delay", 5.0),
            regen_per_s: g("regen_per_s", 45.0),
            charge_move: g("charge_move_mult", 0.5),
            self_dmg: g("self_damage_mult", 0.5),
            armor: g("armor", 0.0),
            rear_mult: g("rear_mult", 1.5),
            front_arc: g("front_arc_deg", 180.0).to_radians(),
            near_bonus: g("near_range_bonus", 0.0),
            vision_bonus: g("vision_bonus", 0.0),
            hearing: g("hearing", 1.0),
        }
    }
}
