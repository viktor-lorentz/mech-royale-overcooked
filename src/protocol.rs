//! mechfight/1 wire types. Everything is optional/defaulted so new server fields never break us.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub type V3 = [f64; 3];

#[derive(Deserialize)]
pub struct Tag {
    #[serde(rename = "type")]
    pub t: String,
}

#[derive(Deserialize, Default, Clone)]
pub struct HelloMap {
    #[serde(default)]
    pub dir: String,
}

#[derive(Deserialize, Default, Clone)]
pub struct Hello {
    #[serde(default)]
    pub slot: u32,
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub map: HelloMap,
    #[serde(default)]
    pub config: Value,
}

#[derive(Deserialize, Default, Clone)]
pub struct WeaponObs {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub cooldown: f64,
    #[serde(default)]
    pub ammo: Option<f64>,
    #[serde(default)]
    pub reload: Option<f64>,
    #[serde(default)]
    pub charge: Option<f64>,
    #[serde(default)]
    pub stats: HashMap<String, f64>,
}

#[derive(Deserialize, Default, Clone)]
pub struct SelfObs {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub pos: V3,
    #[serde(default)]
    pub vel: V3,
    #[serde(default)]
    pub aim: [f64; 2],
    #[serde(default)]
    pub hp: f64,
    #[serde(default)]
    pub max_hp: f64,
    #[serde(default)]
    pub energy: f64,
    #[serde(default)]
    pub max_energy: f64,
    #[serde(default)]
    pub grounded: bool,
    #[serde(default)]
    pub cubes: f64,
    #[serde(default)]
    pub upgrades: Vec<String>,
    #[serde(default)]
    pub in_zone: bool,
    #[serde(default)]
    pub weapons: Vec<Option<WeaponObs>>,
    #[serde(default)]
    pub stats: HashMap<String, f64>,
}

#[derive(Deserialize, Default, Clone)]
pub struct Actor {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub pos: V3,
    #[serde(default)]
    pub vel: V3,
    #[serde(default)]
    pub aim: [f64; 2],
    #[serde(default)]
    pub hp: Option<f64>,
    #[serde(default)]
    pub max_hp: Option<f64>,
    #[serde(default)]
    pub cubes: Option<f64>,
    #[serde(default)]
    pub weapons: Vec<String>,
    #[serde(default)]
    pub charging: bool,
}

#[derive(Deserialize, Default, Clone)]
pub struct Projectile {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub pos: V3,
    #[serde(default)]
    pub vel: V3,
}

#[derive(Deserialize, Default, Clone)]
pub struct Item {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub pos: V3,
    #[serde(default)]
    pub hp: Option<f64>,
}

#[derive(Deserialize, Default, Clone)]
pub struct Pickup {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub pos: V3,
}

#[derive(Deserialize, Default, Clone)]
pub struct Sound {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub bearing: f64,
    #[serde(default)]
    pub dist: String,
    #[serde(default)]
    pub age: f64,
}

#[derive(Deserialize, Default, Clone)]
pub struct Beam {
    #[serde(default)]
    pub by: i64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub path: Vec<V3>,
    #[serde(default)]
    pub age: f64,
}

#[derive(Deserialize, Default, Clone)]
pub struct Event {
    #[serde(rename = "type", default)]
    pub t: String,
    #[serde(default)]
    pub amount: Option<f64>,
    #[serde(default)]
    pub weapon: Option<String>,
    #[serde(default)]
    pub bearing: Option<f64>,
    #[serde(default)]
    pub by: Option<i64>,
    #[serde(default)]
    pub rear: Option<bool>,
    #[serde(default)]
    pub target: Option<i64>,
    #[serde(default)]
    pub effect: Option<String>,
    #[serde(default)]
    pub cubes: Option<f64>,
    #[serde(default)]
    pub id: Option<Value>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Deserialize, Default, Clone)]
pub struct Zone {
    #[serde(default)]
    pub center: [f64; 2],
    #[serde(default)]
    pub radius: f64,
    #[serde(default)]
    pub next_center: [f64; 2],
    #[serde(default)]
    pub next_radius: f64,
    #[serde(default)]
    pub shrink_start: f64,
    #[serde(default)]
    pub shrink_end: f64,
    #[serde(default)]
    pub dps: f64,
}

#[derive(Deserialize, Default, Clone)]
pub struct OfferChoice {
    #[serde(default)]
    pub id: String,
}

#[derive(Deserialize, Default, Clone)]
pub struct Offer {
    #[serde(default)]
    pub choices: Vec<OfferChoice>,
}

#[derive(Deserialize, Default, Clone)]
pub struct SupplyDrop {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub pos: V3,
    #[serde(default)]
    pub radius: f64,
    #[serde(default)]
    pub lands_in: f64,
    #[serde(default)]
    pub capture_s: f64,
    #[serde(default)]
    pub progress: f64,
    #[serde(default)]
    pub holder: Option<i64>,
}

#[derive(Deserialize, Default, Clone)]
pub struct Obs {
    #[serde(default)]
    pub tick: u64,
    #[serde(default)]
    pub t: f64,
    #[serde(default)]
    pub alive: u32,
    #[serde(rename = "self", default)]
    pub me: SelfObs,
    #[serde(default)]
    pub mechs: Vec<Actor>,
    #[serde(default)]
    pub projectiles: Vec<Projectile>,
    #[serde(default)]
    pub crates: Vec<Item>,
    #[serde(default)]
    pub cubes: Vec<Item>,
    #[serde(default)]
    pub pickups: Vec<Pickup>,
    #[serde(default)]
    pub sounds: Vec<Sound>,
    #[serde(default)]
    pub beams: Vec<Beam>,
    #[serde(default)]
    pub events: Vec<Event>,
    #[serde(default)]
    pub zone: Zone,
    #[serde(default)]
    pub offer: Option<Offer>,
    #[serde(default)]
    pub drops: Vec<SupplyDrop>,
}

#[derive(Deserialize, Default, Clone)]
pub struct End {
    #[serde(default)]
    pub place: f64,
    #[serde(default)]
    pub kills: f64,
}

#[derive(Serialize, Default, Clone, Debug)]
pub struct Action {
    pub tick: u64,
    #[serde(rename = "move", skip_serializing_if = "Option::is_none")]
    pub mv: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boost: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jump: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aim: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fire: Option<[bool; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reload: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pick: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub take: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drop: Option<usize>,
}
