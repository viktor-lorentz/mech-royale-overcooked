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
    pub royale_hold_tfrac: f64,
    pub royale_deaths: u32,
    pub drone_r: f64,
    pub drone_hp: f64,
    pub cube_r: f64,
    pub explore_until: f64,
    pub strafe_period: f64,
    pub fire_tol: f64,
}

impl Default for Params {
    fn default() -> Self {
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
            royale_hold_tfrac: 0.5,
            royale_deaths: 3,
            drone_r: 80.0,
            drone_hp: 0.5,
            cube_r: 50.0,
            explore_until: 170.0,
            strafe_period: 1.2,
            fire_tol: 0.9,
        }
    }
}
