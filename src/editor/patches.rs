//! Editor side of patch save/load: the title-bar selector, the browser and
//! the save dialog. Loading applies every parameter through the host so
//! automation and undo see the change.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use truce::prelude::*;
use truce_slint::{KeyboardCapture, KeyboardCaptureMode, PluginContext};

use super::SynthUi;
use crate::patch::{
    DEFAULT_PATCH_NAME, Patch, PatchError, PatchLibrary, is_default_name, validate_name,
};
use crate::plugin::SynthParams;

/// Normalized difference that counts as an edit for the " *" indicator.
const MODIFIED_TOLERANCE: f64 = 1e-4;

#[derive(Debug, PartialEq)]
pub(super) enum SaveOutcome {
    Saved(String),
    /// A patch with this name exists (in the returned spelling); ask first.
    ConfirmOverwrite(String),
}

pub(super) struct PatchController {
    library: PatchLibrary,
    /// Saved patch names; the browser shows "Default" before them.
    names: Vec<String>,
    /// Normalized values of the loaded patch, compared against the live
    /// parameters for the modified indicator.
    reference: Vec<(u32, f64)>,
    /// The persisted patch name `reference` was built for. A mismatch forces
    /// a rebuild, e.g. after the host restores a session.
    reference_name: Option<String>,
    /// Host notifications still to send from the sync closure.
    pending: Vec<(u32, f64)>,
}

impl PatchController {
    pub(super) fn new(library: PatchLibrary) -> Self {
        Self {
            library,
            names: Vec::new(),
            reference: Vec::new(),
            reference_name: None,
            pending: Vec::new(),
        }
    }

    pub(super) fn refresh(&mut self) {
        self.names = self.library.list();
    }

    fn entries(&self) -> Vec<SharedString> {
        std::iter::once(SharedString::from(DEFAULT_PATCH_NAME))
            .chain(self.names.iter().map(SharedString::from))
            .collect()
    }

    /// Browser row of the loaded patch, or -1 if it isn't listed.
    fn current_index(&self, name: &str) -> i32 {
        if is_default_name(name) {
            return 0;
        }
        self.names
            .iter()
            .position(|existing| existing == name)
            .map_or(-1, |index| index as i32 + 1)
    }

    pub(super) fn load_index(
        &mut self,
        params: &SynthParams,
        index: usize,
    ) -> Result<(), PatchError> {
        if index == 0 {
            self.apply(params, &Patch::empty(), "");
            return Ok(());
        }
        let Some(name) = self.names.get(index - 1).cloned() else {
            return Ok(());
        };
        match self.library.load(&name) {
            Ok(patch) => {
                self.apply(params, &patch, &name);
                Ok(())
            }
            Err(error) => {
                self.refresh();
                Err(error)
            }
        }
    }

    /// Load the previous (`-1`) or next (`1`) patch, wrapping around.
    pub(super) fn step(&mut self, params: &SynthParams, delta: i32) -> Result<(), PatchError> {
        self.refresh();
        let count = self.names.len() as i32 + 1;
        let current = self.current_index(&params.patch_name());
        let target = if current < 0 {
            if delta > 0 { 0 } else { count - 1 }
        } else {
            (current + delta).rem_euclid(count)
        };
        self.load_index(params, target as usize)
    }

    fn apply(&mut self, params: &SynthParams, patch: &Patch, name: &str) {
        for (id, value) in patch.normalized_values(params) {
            let changed = params
                .get_normalized(id)
                .is_none_or(|current| (current - value).abs() > f64::EPSILON);
            if changed {
                params.set_normalized(id, value);
                match self.pending.iter_mut().find(|(pending, _)| *pending == id) {
                    Some((_, pending_value)) => *pending_value = value,
                    None => self.pending.push((id, value)),
                }
            }
        }
        params.set_patch_name(name);
        self.snapshot(params, name);
    }

    fn snapshot(&mut self, params: &SynthParams, name: &str) {
        self.reference = Patch::empty()
            .normalized_values(params)
            .into_iter()
            .filter_map(|(id, _)| params.get_normalized(id).map(|value| (id, value)))
            .collect();
        self.reference_name = Some(name.to_owned());
    }

    pub(super) fn save(
        &mut self,
        params: &SynthParams,
        raw_name: &str,
        overwrite: bool,
    ) -> Result<SaveOutcome, PatchError> {
        let name = validate_name(raw_name)?;
        if !overwrite && let Some(existing) = self.library.find(&name) {
            return Ok(SaveOutcome::ConfirmOverwrite(existing));
        }
        let saved = self.library.save(&name, &Patch::capture(params), params)?;
        params.set_patch_name(&saved);
        self.snapshot(params, &saved);
        self.refresh();
        Ok(SaveOutcome::Saved(saved))
    }

    /// Delete a saved patch. Deleting the loaded patch keeps its name and
    /// sound so the user can still re-save it.
    pub(super) fn delete_index(&mut self, index: usize) -> Result<(), PatchError> {
        let Some(name) = index
            .checked_sub(1)
            .and_then(|i| self.names.get(i))
            .cloned()
        else {
            return Ok(());
        };
        let result = self.library.delete(&name);
        self.refresh();
        result
    }

    pub(super) fn is_modified(&mut self, params: &SynthParams) -> bool {
        let name = params.patch_name();
        if self.reference_name.as_deref() != Some(name.as_str()) {
            self.rebuild_reference(params, &name);
        }
        self.reference.iter().any(|&(id, value)| {
            params
                .get_normalized(id)
                .is_some_and(|current| (current - value).abs() > MODIFIED_TOLERANCE)
        })
    }

    fn rebuild_reference(&mut self, params: &SynthParams, name: &str) {
        let patch = if is_default_name(name) {
            Some(Patch::empty())
        } else {
            self.library.load(name).ok()
        };
        match patch {
            Some(patch) => {
                self.reference = patch.normalized_values(params);
                self.reference_name = Some(name.to_owned());
            }
            // The file is gone or unreadable: treat the current sound as the
            // patch rather than flagging everything as modified.
            None => self.snapshot(params, name),
        }
    }

    fn take_pending(&mut self) -> Vec<(u32, f64)> {
        std::mem::take(&mut self.pending)
    }
}

fn display_name(name: &str) -> SharedString {
    if is_default_name(name) {
        DEFAULT_PATCH_NAME.into()
    } else {
        name.into()
    }
}

pub(super) fn wire(
    ui: &SynthUi,
    state: &PluginContext<SynthParams>,
    controller: &Rc<RefCell<PatchController>>,
) {
    let list_model = Rc::new(VecModel::from(controller.borrow().entries()));
    ui.set_patches(ModelRc::from(list_model.clone()));
    let refresh_list = {
        let controller = controller.clone();
        move || {
            let entries = controller.borrow().entries();
            list_model.set_vec(entries);
        }
    };
    let report = {
        let ui = ui.as_weak();
        move |result: Result<(), PatchError>| {
            if let Some(ui) = ui.upgrade() {
                let message = result.err().map(|e| e.to_string()).unwrap_or_default();
                ui.set_patch_error(message.into());
            }
        }
    };

    {
        let ui_weak = ui.as_weak();
        let controller = controller.clone();
        let refresh_list = refresh_list.clone();
        ui.on_patch_browser_toggled(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            if ui.get_patch_browser_open() {
                ui.set_patch_browser_open(false);
                return;
            }
            controller.borrow_mut().refresh();
            refresh_list();
            ui.set_patch_error(SharedString::new());
            ui.set_patch_browser_open(true);
        });
    }
    {
        let state = state.clone();
        let controller = controller.clone();
        let refresh_list = refresh_list.clone();
        let report = report.clone();
        ui.on_patch_selected(move |index| {
            let result = controller
                .borrow_mut()
                .load_index(state.params(), index.max(0) as usize);
            refresh_list();
            report(result);
        });
    }
    for delta in [-1, 1] {
        let state = state.clone();
        let controller = controller.clone();
        let refresh_list = refresh_list.clone();
        let report = report.clone();
        let step = move || {
            let result = controller.borrow_mut().step(state.params(), delta);
            refresh_list();
            report(result);
        };
        if delta < 0 {
            ui.on_patch_previous(step);
        } else {
            ui.on_patch_next(step);
        }
    }
    {
        let controller = controller.clone();
        let refresh_list = refresh_list.clone();
        ui.on_patch_deleted(move |index| {
            let result = controller.borrow_mut().delete_index(index.max(0) as usize);
            refresh_list();
            report(result);
        });
    }
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        ui.on_patch_save_requested(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let name = state.params().patch_name();
            ui.set_save_name(if is_default_name(&name) {
                SharedString::new()
            } else {
                name.into()
            });
            ui.set_save_error(SharedString::new());
            ui.set_save_confirm_overwrite(false);
            ui.set_save_existing_name(SharedString::new());
            ui.set_save_dialog_open(true);
        });
    }
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let controller = controller.clone();
        ui.on_patch_saved(move |name, overwrite| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let result = controller
                .borrow_mut()
                .save(state.params(), &name, overwrite);
            match result {
                Ok(SaveOutcome::Saved(_)) => {
                    refresh_list();
                    ui.set_patch_error(SharedString::new());
                    ui.set_save_dialog_open(false);
                }
                Ok(SaveOutcome::ConfirmOverwrite(existing)) => {
                    ui.set_save_error(SharedString::new());
                    ui.set_save_existing_name(existing.into());
                    ui.set_save_confirm_overwrite(true);
                }
                Err(error) => {
                    ui.set_save_confirm_overwrite(false);
                    ui.set_save_error(error.to_string().into());
                }
            }
        });
    }
}

/// Per frame: notify the host of loaded values, refresh the title bar and
/// choose which keys the editor window consumes.
pub(super) fn sync(
    ui: &SynthUi,
    state: &PluginContext<SynthParams>,
    controller: &RefCell<PatchController>,
    keyboard: &KeyboardCapture,
) {
    let mut controller = controller.borrow_mut();
    for (id, value) in controller.take_pending() {
        state.automate(id, value);
    }
    let name = state.params().patch_name();
    ui.set_patch_name(display_name(&name));
    ui.set_patch_modified(controller.is_modified(state.params()));
    ui.set_current_patch(controller.current_index(&name));
    keyboard.set(if ui.get_save_dialog_open() {
        KeyboardCaptureMode::All
    } else if ui.get_patch_browser_open() {
        KeyboardCaptureMode::Escape
    } else {
        KeyboardCaptureMode::None
    });
}
