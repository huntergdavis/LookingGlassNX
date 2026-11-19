//! One thread-local for the whole game.
//!
//! libnx hands out twelve TLS slots per process: `threadTlsAlloc` rejects any slot index above
//! `0x0b` (`cmp w19, #0xb; b.hi`), and `pthread_key_create` turns that rejection into an error,
//! which std reports as `fatal runtime error: out of TLS keys, aborting`.
//!
//! std spends **one key per `thread_local!` static, on first touch, with no exceptions** — this
//! target has no `has-thread-local`, so std uses the key/destructor implementation, and even a
//! `Cell<f32>` cache carries a `destroy_value` entry. This crate declared eighteen such statics
//! (fifteen reachable on Horizon; the vendored miniquad adds one more), so the thirteenth key
//! allocation aborted. Build 13d died exactly there, inside `pickup_effects::Effect::load`.
//! Keys are per *process*, not per thread: starting another thread costs no key, only the set of
//! statics first touched matters.
//!
//! Measured in the linked binary: 23 `destroy_value` statics before this change, 10 after — one
//! from here, the rest std and dependencies — against a limit of 12. That is 2 slots of margin,
//! not unlimited room, so do not spend them.
//!
//! Every such cache therefore lives in this one struct behind a single `thread_local!`, and each
//! module keeps its old `NAME.with(|cell| ..)` call shape through [`Field`] / [`CellField`]. The
//! game still gets thread-local storage semantics; it now costs one slot instead of nineteen.
//!
//! Adding a cache: add the field here, initialise it in `State::new`, and declare the module-level
//! accessor. Do **not** add another `thread_local!` — that is how this bug came back the first time.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Weak;

use macroquad::miniquad::TextureId;
use macroquad::prelude::{Material, Texture2D};
use serde_json::Value;

pub(crate) struct State {
    pub(crate) ui_shared: RefCell<Weak<crate::ui::Ui>>,
    pub(crate) ui_probe_previous: RefCell<Value>,
    pub(crate) ui_probe_notes: RefCell<BTreeMap<String, Value>>,
    pub(crate) frozen_cache: RefCell<Weak<crate::frozen::Art>>,
    pub(crate) attack_fx_cache: RefCell<Weak<crate::npc::attack_fx::Art>>,
    pub(crate) electric_cache: RefCell<Weak<crate::electric::Art>>,
    pub(crate) skin_material: RefCell<Weak<Material>>,
    pub(crate) burst_material: RefCell<Weak<Material>>,
    pub(crate) particle_materials: RefCell<Vec<Weak<Material>>>,
    pub(crate) loot_materials: RefCell<[Weak<Material>; 2]>,
    pub(crate) frame: RefCell<crate::render_fx::Frame>,
    pub(crate) skin_opacity: Cell<f32>,
    pub(crate) surfaces: RefCell<Vec<(TextureId, Weak<crate::render_fx::Surface>)>>,
    pub(crate) lights: RefCell<Vec<crate::lighting::Light>>,
    pub(crate) shadow: RefCell<Option<Texture2D>>,
    pub(crate) flare: RefCell<Option<Texture2D>>,
    pub(crate) croquet: RefCell<Vec<crate::lighting::Light>>,
    pub(crate) profile_samples: RefCell<BTreeMap<&'static str, Vec<f64>>>,
}

impl State {
    fn new() -> Self {
        Self {
            ui_shared: RefCell::new(Weak::new()),
            ui_probe_previous: RefCell::new(Value::Null),
            ui_probe_notes: RefCell::new(BTreeMap::new()),
            frozen_cache: RefCell::new(Weak::new()),
            attack_fx_cache: RefCell::new(Weak::new()),
            electric_cache: RefCell::new(Weak::new()),
            skin_material: RefCell::new(Weak::new()),
            burst_material: RefCell::new(Weak::new()),
            particle_materials: RefCell::new(Vec::new()),
            loot_materials: RefCell::new([Weak::new(), Weak::new()]),
            frame: RefCell::new(crate::render_fx::Frame::default()),
            skin_opacity: Cell::new(1.),
            surfaces: RefCell::new(Vec::new()),
            lights: RefCell::new(Vec::new()),
            shadow: RefCell::new(None),
            flare: RefCell::new(None),
            croquet: RefCell::new(Vec::new()),
            profile_samples: RefCell::new(BTreeMap::new()),
        }
    }
}

thread_local! {
    static STATE: State = State::new();
}

/// Borrow `RefCell`-shaped state. Reentrant with respect to other fields, exactly like separate
/// `thread_local!`s were; two immutable borrows of one field are still fine, and a mutable borrow
/// alongside any other still panics, exactly as it did before.
pub(crate) struct Field<T>(for<'a> fn(&'a State) -> &'a RefCell<T>);

impl<T> Field<T> {
    pub(crate) const fn new(get: for<'a> fn(&'a State) -> &'a RefCell<T>) -> Self {
        Self(get)
    }

    pub(crate) fn with<R>(&self, f: impl FnOnce(&RefCell<T>) -> R) -> R {
        STATE.with(|state| f((self.0)(state)))
    }
}

/// Same, for `Cell`-shaped state (`Cell::with` took a `Cell`, not a `RefCell`).
pub(crate) struct CellField<T>(for<'a> fn(&'a State) -> &'a Cell<T>);

impl<T> CellField<T> {
    pub(crate) const fn new(get: for<'a> fn(&'a State) -> &'a Cell<T>) -> Self {
        Self(get)
    }

    pub(crate) fn with<R>(&self, f: impl FnOnce(&Cell<T>) -> R) -> R {
        STATE.with(|state| f((self.0)(state)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_independent_and_reentrant_across_fields() {
        let lights: Field<Vec<crate::lighting::Light>> = Field::new(|s| &s.lights);
        let shadow: Field<Option<Texture2D>> = Field::new(|s| &s.shadow);
        lights.with(|l| {
            shadow.with(|s| {
                // A nested borrow of a *different* field is what the old TLS blocks allowed.
                assert!(s.borrow().is_none());
                assert!(l.borrow().is_empty());
            });
        });
    }

    #[test]
    fn same_type_fields_do_not_alias() {
        // `lights` and `croquet` are both `RefCell<Vec<Light>>`, so a copy-paste accessor could
        // point at the wrong one and still type-check. Two immutable borrows would hide that.
        let lights: Field<Vec<crate::lighting::Light>> = Field::new(|s| &s.lights);
        let croquet: Field<Vec<crate::lighting::Light>> = Field::new(|s| &s.croquet);
        let lights_ptr = lights.with(|l| l as *const _ as usize);
        let croquet_ptr = croquet.with(|c| c as *const _ as usize);
        assert_ne!(lights_ptr, croquet_ptr, "two accessors share one field");
    }

    #[test]
    fn cell_field_round_trips() {
        let opacity: CellField<f32> = CellField::new(|s| &s.skin_opacity);
        opacity.with(|o| o.set(0.5));
        opacity.with(|o| assert_eq!(o.get(), 0.5));
        opacity.with(|o| o.set(1.0));
    }
}
