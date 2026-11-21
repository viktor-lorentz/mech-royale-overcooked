//! Every behaviour tunable in one place. Defaults = the shipped bot; the simulator can override them per run.
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct Params {
    pub laser_band: [f64; 2],
    pub light_band: [f64; 2],
    pub mortar_band: [f64; 2],
    pub gun_band: [f64; 2],
    pub hurt_hp: f64,
    pub cover_exit_hp: f64,
    pub threat_r: f64,
    pub outnumber_n: usize, // 0 = off
    pub outnumber_r: f64,
    pub low_hp: f64,
    pub finish_tfrac: f64,
    pub finish_hp: f64,
    pub push_radial: f64,
    pub retreat_radial: f64,
    pub royale_hp_w: f64,
    pub gauntlet_hp_w: f64,
    pub royale_hold_tfrac: f64,
    pub royale_deaths: u32,
    pub infight_n: u32, // 0 = off
    pub drone_r: f64,
    pub drone_hp: f64,
    pub drone_target_r: f64,
    pub drone_avoid: f64,
    pub cube_r: f64,
    pub crate_r: f64,
    pub crate_r_unarmed: f64,
    pub pickup_r_unarmed: f64,
    pub pickup_r_armed: f64,
    pub loot_enemy_r: f64,
    pub explore_until: f64,
    pub strafe_period: f64,
    pub fire_tol: f64,
    pub attacker_mem: f64,
    pub attacker_switch: f64,
    pub face_unseen: bool,
    pub survival_upgrades: bool,
    pub cover_window: f64,
}

/// Which strategy preset ships. Each submitted variant is a commit that changes only this line.
pub const VARIANT: &str = "controlled";

impl Default for Params {
    fn default() -> Self {
        Params::preset(VARIANT)
    }
}

impl Params {
    pub fn preset(name: &str) -> Self {
        let b = Params::base();
        match name {
            // A) survivor: fight only 1v1 with an edge, from cover; heal to full; survival upgrades; hoard cubes
            "survivor" => Params { hurt_hp: 0.6, cover_exit_hp: 1.0, cover_window: 30.0, outnumber_n: 2, outnumber_r: 200.0, push_radial: 0.0, finish_tfrac: 0.0, survival_upgrades: true, cube_r: 80.0, drone_hp: 0.8, ..b },
            // B) controlled killer: never a second fight until full HP, never chase, cover when >1 hunter visible
            "controlled" => Params { cover_exit_hp: 1.0, cover_window: 30.0, outnumber_n: 2, outnumber_r: 200.0, push_radial: 0.0, finish_tfrac: 0.0, ..b },
            // C) current bot with a much higher disengage/heal threshold
            "heal" => Params { hurt_hp: 0.75, cover_exit_hp: 0.95, ..b },
            _ => b,
        }
    }

    pub fn base() -> Self {
        Params {
            laser_band: [80.0, 150.0],
            light_band: [20.0, 38.0],
            mortar_band: [60.0, 140.0],
            gun_band: [40.0, 90.0],
            hurt_hp: 0.5,
            cover_exit_hp: 0.8,
            threat_r: 160.0,
            outnumber_n: 0,
            outnumber_r: 150.0,
            low_hp: 0.3,
            finish_tfrac: 0.35,
            finish_hp: 0.5,
            push_radial: 0.6,
            retreat_radial: 0.8,
            royale_hp_w: 160.0,
            gauntlet_hp_w: 0.0,
            royale_hold_tfrac: 0.5,
            royale_deaths: 3,
            infight_n: 2,
            drone_r: 80.0,
            drone_hp: 0.5,
            drone_target_r: 48.0,
            drone_avoid: 0.8,
            cube_r: 50.0,
            crate_r: 60.0,
            crate_r_unarmed: 60.0,
            pickup_r_unarmed: 250.0,
            pickup_r_armed: 180.0,
            loot_enemy_r: 0.0,
            explore_until: 170.0,
            strafe_period: 1.2,
            fire_tol: 0.9,
            attacker_mem: 2.0,
            attacker_switch: 1.5,
            face_unseen: true,
            survival_upgrades: false,
            cover_window: 8.0,
        }
    }
}
