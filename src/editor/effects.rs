use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};
use truce::prelude::*;
use truce_slint::{PluginContext, SyncFn};

use super::{EffectRow, SynthUi, filter_type_to_normalized, sync_model};
use crate::engine::MAX_EFFECTS;
use crate::plugin::SynthParams;
use crate::plugin::effects::effect_ids;

type Edits = Rc<RefCell<Vec<(u32, f64)>>>;

fn set(state: &PluginContext<SynthParams>, edits: &Edits, id: u32, value: f64) {
    state.params().set_normalized(id, value);
    let mut edits = edits.borrow_mut();
    if let Some((_, pending)) = edits.iter_mut().find(|(pending, _)| *pending == id) {
        *pending = value;
    } else {
        edits.push((id, value));
    }
}

fn order(state: &PluginContext<SynthParams>, edits: &Edits, slots: &[usize]) {
    for (position, &slot) in slots.iter().enumerate() {
        set(state, edits, effect_ids(slot)[5], position as f64 / 31.0);
    }
}

fn add(state: &PluginContext<SynthParams>, edits: &Edits) {
    let chain = state.params().effect_chain();
    let Some(slot) = (0..MAX_EFFECTS).find(|slot| !chain.slots().contains(slot)) else {
        return;
    };
    let ids = effect_ids(slot);
    let infos = state.params().param_infos();
    for id in &ids[..4] {
        let info = infos
            .iter()
            .find(|info| info.id == *id)
            .expect("effect parameter");
        set(state, edits, *id, info.range.normalize(info.default_plain));
    }
    let mut slots = chain.slots().to_vec();
    slots.push(slot);
    order(state, edits, &slots);
    set(state, edits, ids[4], 1.0);
}

fn remove(state: &PluginContext<SynthParams>, edits: &Edits, slot: usize) {
    if !state.params().effect_chain().slots().contains(&slot) {
        return;
    }
    set(state, edits, effect_ids(slot)[4], 0.0);
    for source in (0..super::MAX_LFOS)
        .map(super::Modulator::Lfo)
        .chain((0..super::MAX_ENVELOPES).map(super::Modulator::Envelope))
    {
        let routes = super::read_modulator_routes(state, source);
        let (destinations, amounts) = source.routing();
        for (index, route) in routes.iter().enumerate() {
            if route
                .destination
                .and_then(|destination| destination.effect())
                == Some(slot)
            {
                set(state, edits, destinations[index].into(), 0.0);
                set(state, edits, amounts[index].into(), 0.5);
                set(state, edits, source.filters()[index].into(), 0.0);
            }
        }
    }
    let chain = state.params().effect_chain();
    order(state, edits, chain.slots());
}

fn move_effect(state: &PluginContext<SynthParams>, edits: &Edits, slot: usize, target: usize) {
    let mut slots = state.params().effect_chain().slots().to_vec();
    let Some(source) = slots.iter().position(|&identity| identity == slot) else {
        return;
    };
    if target >= slots.len() || target == source {
        return;
    }
    slots.remove(source);
    slots.insert(target, slot);
    order(state, edits, &slots);
}

pub(super) fn wire(ui: &SynthUi, state: &PluginContext<SynthParams>) -> SyncFn<SynthParams> {
    let edits: Edits = Rc::new(RefCell::new(Vec::new()));
    let callback_state = state.clone();
    let callback_edits = edits.clone();
    let weak = ui.as_weak();
    ui.on_effect_add(move || {
        add(&callback_state, &callback_edits);
        weak.upgrade()
            .expect("live editor")
            .set_scroll_effects_to_end(true);
    });
    let callback_state = state.clone();
    let callback_edits = edits.clone();
    ui.on_effect_remove(move |slot| {
        if let Ok(slot) = usize::try_from(slot) {
            remove(&callback_state, &callback_edits, slot);
        }
    });
    let callback_state = state.clone();
    let callback_edits = edits.clone();
    ui.on_effect_moved(move |slot, target| {
        if let (Ok(slot), Ok(target)) = (usize::try_from(slot), usize::try_from(target)) {
            move_effect(&callback_state, &callback_edits, slot, target);
        }
    });
    let callback_state = state.clone();
    let callback_edits = edits.clone();
    ui.on_effect_kind_selected(move |slot, kind| {
        if let Ok(slot) = usize::try_from(slot) {
            assert!(slot < MAX_EFFECTS);
            set(
                &callback_state,
                &callback_edits,
                effect_ids(slot)[0],
                filter_type_to_normalized(kind),
            );
        }
    });
    let callback_state = state.clone();
    ui.on_effect_changed(move |slot, control, value| {
        let slot = usize::try_from(slot).expect("effect slot");
        let control = usize::try_from(control).expect("effect control");
        assert!(slot < MAX_EFFECTS && control < 3);
        callback_state
            .params()
            .set_normalized(effect_ids(slot)[control + 1], f64::from(value));
    });
    let callback_state = state.clone();
    let callback_edits = edits.clone();
    ui.on_effect_released(move |slot, control| {
        let slot = usize::try_from(slot).expect("effect slot");
        let control = usize::try_from(control).expect("effect control");
        assert!(slot < MAX_EFFECTS && control < 3);
        let id = effect_ids(slot)[control + 1];
        set(
            &callback_state,
            &callback_edits,
            id,
            f64::from(callback_state.get_param(id)),
        );
    });

    let model = Rc::new(VecModel::from(Vec::<EffectRow>::new()));
    ui.set_effects(ModelRc::from(model.clone()));
    Box::new(move |state| {
        for (id, value) in edits.borrow_mut().drain(..) {
            state.automate(id, value);
        }
        let rows = state
            .params()
            .effect_chain()
            .slots()
            .iter()
            .map(|&slot| {
                let ids = effect_ids(slot);
                EffectRow {
                    slot: slot as i32,
                    kind: (state.get_param(ids[0]) * 2.0).round() as i32,
                    cutoff: state.get_param(ids[1]),
                    cutoff_text: state.format_param(ids[1]).into(),
                    q: state.get_param(ids[2]),
                    q_text: state.format_param(ids[2]).into(),
                    mix: state.get_param(ids[3]),
                    mix_text: state.format_param(ids[3]).into(),
                }
            })
            .collect();
        sync_model(&model, rows);
    })
}
