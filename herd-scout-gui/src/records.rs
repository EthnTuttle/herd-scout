//! Records tab — Animal / Group / Land / Equipment list + create form.
//!
//! Phase 4 of plan-fms-schema-and-records-2026-06-02. MVP scope: list
//! assets across all four kinds with a kind filter, plus a single
//! create-form that picks the kind via dropdown and submits. Logs
//! and per-asset detail views land in a follow-up; the IPC plumbing
//! they'd need is already in place.
//!
//! ## Data flow
//!
//! 1. On tab activation, we send `ClientMsg::FmsListAssets` for every
//!    kind (four small RPCs) and stash the most recent reply per kind.
//! 2. On `ServerMsg::FmsAssetList` we update the cached list for that
//!    kind and request a repaint.
//! 3. On `ServerMsg::FmsChange` with `entity_hint == Some("asset")` we
//!    re-issue the affected list query (cheap) so the UI stays in
//!    sync without full schema awareness.
//! 4. The create-form fires `ClientMsg::FmsCreateAsset`; the `FmsAsset`
//!    reply arrives as part of the next list-refresh because the
//!    change-bridge already pushed an `FmsChange`.
//!
//! Concurrency: every IPC reply lands in `RecordsState` via the GUI's
//! `apply_msg` dispatcher; the egui paint loop reads it under a
//! `parking_lot::RwLock`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use eframe::egui;
use herd_scout_ipc::{AssetKindWire, AssetWire, ClientMsg, LogWire};
use parking_lot::RwLock;

use crate::ipc::client::IpcClientHandle;

#[derive(Debug, Default)]
pub struct RecordsState {
    /// Cached list per kind. `None` = never loaded.
    pub animals: RwLock<Option<Vec<AssetWire>>>,
    pub groups: RwLock<Option<Vec<AssetWire>>>,
    pub lands: RwLock<Option<Vec<AssetWire>>>,
    pub equipment: RwLock<Option<Vec<AssetWire>>>,
    /// Most recent FmsError that targeted a Records-tab request, for
    /// inline error display. Cleared on success.
    pub last_error: RwLock<Option<String>>,
    /// Phase 3c: most recent log-search reply. `None` = never run a
    /// search this session. Empty `Vec` = ran but no hits.
    pub search_results: RwLock<Option<Vec<LogWire>>>,
    /// Set by the IPC reader on every `FmsChange`; the egui paint loop
    /// observes it on the next frame, issues a `refresh_all`, and
    /// clears it. Decouples the reader (no `IpcClientHandle`) from
    /// the writer (only the paint loop holds the handle).
    pub refresh_pending: RwLock<bool>,
    /// Monotonic request-id generator. Replies carry this back so
    /// out-of-order responses stay correlated. We currently match by
    /// kind, not request-id — kept for future per-request UX.
    pub next_request_id: AtomicU64,
}

impl RecordsState {
    pub fn arc() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn next_id(&self) -> u64 {
        self.next_request_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Apply an incoming asset list reply.
    pub fn apply_list(&self, kind: AssetKindWire, assets: Vec<AssetWire>) {
        match kind {
            AssetKindWire::Animal => *self.animals.write() = Some(assets),
            AssetKindWire::Group => *self.groups.write() = Some(assets),
            AssetKindWire::Land => *self.lands.write() = Some(assets),
            AssetKindWire::Equipment => *self.equipment.write() = Some(assets),
        }
    }

    /// Drains the `refresh_pending` flag. Called by the egui paint
    /// loop; if `true`, the caller issues `refresh_all`.
    pub fn drain_refresh(&self) -> bool {
        let mut g = self.refresh_pending.write();
        let pending = *g;
        *g = false;
        pending
    }

    /// Send a list query for every kind. Called on tab activation +
    /// after change events.
    pub fn refresh_all(&self, handle: &IpcClientHandle) {
        for kind in [
            AssetKindWire::Animal,
            AssetKindWire::Group,
            AssetKindWire::Land,
            AssetKindWire::Equipment,
        ] {
            let request_id = self.next_id();
            handle.try_send(ClientMsg::FmsListAssets {
                request_id,
                kind,
                include_archived: false,
            });
        }
    }

    /// Surface an error message in the Records UI.
    pub fn set_error(&self, message: String) {
        *self.last_error.write() = Some(message);
    }

    /// Phase 3c: stash a log-search reply. The IPC dispatcher routes
    /// `ServerMsg::FmsLogList { asset_id: "", logs }` here.
    pub fn apply_search_results(&self, logs: Vec<LogWire>) {
        *self.search_results.write() = Some(logs);
    }
}

/// Per-frame UI state for the Records tab. Held inside `App` and
/// reset when the tab loses focus.
#[derive(Debug)]
pub struct RecordsUi {
    /// Currently-selected kind in the list filter.
    pub filter_kind: AssetKindWire,
    /// Open create-form state (when `Some`, the modal/inline form is
    /// rendered).
    pub create_open: bool,
    pub create_kind: AssetKindWire,
    pub create_name: String,
    /// True when the user has activated the tab at least once. Used
    /// to drive the initial list-refresh.
    pub primed: bool,
    /// Phase 3c: full-text search box buffer.
    pub search_query: String,
}

impl Default for RecordsUi {
    fn default() -> Self {
        Self {
            filter_kind: AssetKindWire::Animal,
            create_open: false,
            create_kind: AssetKindWire::Animal,
            create_name: String::new(),
            primed: false,
            search_query: String::new(),
        }
    }
}

/// Renders the Records tab into the central panel.
///
/// Returns `true` when the user issued a write (so callers can mark
/// other UI as dirty if desired). Today we ignore the return value —
/// the change-bridge handles refresh.
pub fn render(
    ui: &mut egui::Ui,
    state: &Arc<RecordsState>,
    ui_state: &mut RecordsUi,
    handle: &IpcClientHandle,
) -> bool {
    if !ui_state.primed {
        state.refresh_all(handle);
        ui_state.primed = true;
    }

    let mut wrote = false;

    ui.heading("Records");
    ui.add_space(4.0);

    // Top row: kind filter + create-button.
    ui.horizontal(|ui| {
        ui.label("Kind:");
        kind_combo("filter_kind", ui, &mut ui_state.filter_kind);

        ui.add_space(20.0);
        if ui.button("+ New asset").clicked() {
            ui_state.create_open = true;
            ui_state.create_kind = ui_state.filter_kind;
            ui_state.create_name.clear();
        }
        if ui.button("Refresh").clicked() {
            state.refresh_all(handle);
        }
    });

    if let Some(err) = state.last_error.read().clone() {
        ui.colored_label(egui::Color32::LIGHT_RED, format!("Error: {err}"));
    }

    // Phase 3c: full-text search across log notes.
    ui.horizontal(|ui| {
        ui.label("Search logs:");
        let edit = ui.add(
            egui::TextEdit::singleline(&mut ui_state.search_query)
                .hint_text("e.g. thorn OR limping")
                .desired_width(280.0),
        );
        let do_search = (edit.lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            || ui.button("Search").clicked();
        if do_search && !ui_state.search_query.trim().is_empty() {
            handle.try_send(ClientMsg::FmsSearchLogs {
                request_id: state.next_id(),
                query: ui_state.search_query.trim().to_string(),
                limit: 50,
            });
        }
        if ui.button("Clear").clicked() {
            ui_state.search_query.clear();
            *state.search_results.write() = None;
        }
    });

    if let Some(results) = state.search_results.read().clone() {
        ui.label(format!("{} hit(s)", results.len()));
        egui::ScrollArea::vertical()
            .id_salt("search_results_scroll")
            .max_height(160.0)
            .show(ui, |ui| {
                if results.is_empty() {
                    ui.label("(no matching logs)");
                } else {
                    egui::Grid::new("search_results_grid")
                        .num_columns(3)
                        .spacing([16.0, 4.0])
                        .striped(true)
                        .show(ui, |ui| {
                            ui.strong("Kind");
                            ui.strong("Notes");
                            ui.strong("Refs");
                            ui.end_row();
                            for log in &results {
                                ui.label(log_kind_label(&log.kind));
                                ui.label(truncate(&log.notes, 80));
                                ui.label(format!("{}", log.asset_refs.len()));
                                ui.end_row();
                            }
                        });
                }
            });
    }

    ui.separator();

    // Inline create form. We use a collapsing window so the form takes
    // no space when closed; egui-modal would be heavier than needed.
    if ui_state.create_open {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.heading("New asset");
            ui.horizontal(|ui| {
                ui.label("Kind:");
                kind_combo("create_kind", ui, &mut ui_state.create_kind);
            });
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.text_edit_singleline(&mut ui_state.create_name);
            });
            ui.horizontal(|ui| {
                let can_create = !ui_state.create_name.trim().is_empty();
                if ui
                    .add_enabled(can_create, egui::Button::new("Create"))
                    .clicked()
                {
                    handle.try_send(ClientMsg::FmsCreateAsset {
                        request_id: state.next_id(),
                        kind: ui_state.create_kind,
                        name: ui_state.create_name.trim().to_string(),
                    });
                    ui_state.create_open = false;
                    ui_state.create_name.clear();
                    wrote = true;
                }
                if ui.button("Cancel").clicked() {
                    ui_state.create_open = false;
                }
            });
        });
        ui.separator();
    }

    // List view for the selected kind.
    let snapshot = match ui_state.filter_kind {
        AssetKindWire::Animal => state.animals.read().clone(),
        AssetKindWire::Group => state.groups.read().clone(),
        AssetKindWire::Land => state.lands.read().clone(),
        AssetKindWire::Equipment => state.equipment.read().clone(),
    };

    match snapshot {
        None => {
            ui.label("Loading…");
        }
        Some(assets) if assets.is_empty() => {
            ui.label("No records yet. Click + New asset to add one.");
        }
        Some(assets) => {
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("assets_grid")
                    .num_columns(3)
                    .spacing([24.0, 6.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("Name");
                        ui.strong("ID");
                        ui.strong("Tags");
                        ui.end_row();
                        for asset in &assets {
                            ui.label(&asset.name);
                            ui.label(short_id(&asset.id));
                            ui.label(format!("{}", asset.tags.len()));
                            ui.end_row();
                        }
                    });
            });
        }
    }

    wrote
}

fn kind_combo(id: &str, ui: &mut egui::Ui, kind: &mut AssetKindWire) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(kind_label(*kind))
        .show_ui(ui, |ui| {
            for k in [
                AssetKindWire::Animal,
                AssetKindWire::Group,
                AssetKindWire::Land,
                AssetKindWire::Equipment,
            ] {
                ui.selectable_value(kind, k, kind_label(k));
            }
        });
}

fn kind_label(k: AssetKindWire) -> &'static str {
    match k {
        AssetKindWire::Animal => "Animal",
        AssetKindWire::Group => "Group",
        AssetKindWire::Land => "Land",
        AssetKindWire::Equipment => "Equipment",
    }
}

fn short_id(id: &str) -> String {
    if id.len() > 12 {
        format!("{}…", &id[..12])
    } else {
        id.to_string()
    }
}

fn log_kind_label(k: &herd_scout_ipc::LogKindWire) -> &'static str {
    use herd_scout_ipc::LogKindWire as L;
    match k {
        L::Observation => "Observation",
        L::Medical => "Medical",
        L::Movement => "Movement",
        L::Weight => "Weight",
        L::Birth => "Birth",
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}
