use crate::{
    catalog::{self, Facets, Home, Page, Request, SearchFilters, SearchResults, SearchScope},
    playback::Selection,
    protocol::{self, Item, Session},
    service::{Command as PlaybackCommand, Service},
    state::{Model, Section},
};
use bluefin_contract::{Capabilities, MusicQueue, PROTOCOL_VERSION, UpdateMusicQueue};
use eframe::egui;
#[path = "artwork.rs"]
mod artwork;
#[path = "theme.rs"]
mod theme;
use futures_util::future::{AbortHandle, Abortable};
use serde_json::Value;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[cfg(target_arch = "wasm32")]
#[path = "web_dom.rs"]
pub(crate) mod web_dom;
struct SignInDrain(Arc<AtomicUsize>);
impl Drop for SignInDrain {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

#[cfg(not(target_arch = "wasm32"))]
trait Work: std::future::Future<Output = Result<Outcome, String>> + Send + 'static {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: std::future::Future<Output = Result<Outcome, String>> + Send + 'static> Work for T {}
#[cfg(target_arch = "wasm32")]
trait Work: std::future::Future<Output = Result<Outcome, String>> + 'static {}
#[cfg(target_arch = "wasm32")]
impl<T: std::future::Future<Output = Result<Outcome, String>> + 'static> Work for T {}

enum Outcome {
    Login(Session),
    Connected(protocol::ConnectedServer),
    DeviceSignIn(protocol::DeviceSignIn),
    Waiting,
    #[cfg(target_arch = "wasm32")]
    MeshStep(crate::mesh::PairingStep),
    #[cfg(target_arch = "wasm32")]
    MeshEmpty,
    Data(&'static str, Value),
    Home(Home),
    Rows(Page, bool),
    Search(SearchResults, Option<Facets>),
    SearchMore(usize, Page),
}
type Completion = (u64, Result<Outcome, String>);
struct PreviousShelf {
    rows: Vec<Item>,
    detail: Option<Item>,
    page: Page,
    request: Option<Request>,
}
pub struct Bluefin {
    styled: bool,
    model: Model,
    server: String,
    name: String,
    password: String,
    connected_server: Option<protocol::ConnectedServer>,
    device_sign_in: Option<protocol::DeviceSignIn>,
    query: String,
    filters: SearchFilters,
    facets: Option<Facets>,
    search_results: SearchResults,
    home: Home,
    page: Page,
    catalog_request: Option<Request>,
    artwork: artwork::Cache,
    brand_texture: Option<egui::TextureHandle>,
    #[cfg(target_arch = "wasm32")]
    mesh_code: String,
    #[cfg(target_arch = "wasm32")]
    mesh_name: String,
    #[cfg(target_arch = "wasm32")]
    mesh_step: Option<crate::mesh::PairingStep>,
    #[cfg(target_arch = "wasm32")]
    mesh_invite: Option<crate::mesh::IssuedInvite>,
    #[cfg(target_arch = "wasm32")]
    mesh_invite_check: u64,
    previous_shelves: Vec<PreviousShelf>,
    error: String,
    tx: Sender<Completion>,
    rx: Receiver<Completion>,
    pending: Option<AbortHandle>,
    auth_drains: Arc<AtomicUsize>,
    discovery: Option<bluefin_contract::discovery::DiscoveryPage>,
    form: Option<bluefin_contract::discovery::ActionForm>,
    selection: Vec<String>,
    confirmed: bool,
    peer_id: String,
    peer_address: String,
    playback: Option<Service>,
    retired: Vec<Service>,
    #[cfg(not(target_arch = "wasm32"))]
    video: Option<egui::TextureHandle>,
    #[cfg(not(target_arch = "wasm32"))]
    video_session: Option<String>,
}
impl Default for Bluefin {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            styled: false,
            model: Model::default(),
            server: web_hosted_server(),
            name: String::new(),
            password: String::new(),
            connected_server: None,
            device_sign_in: None,
            query: String::new(),
            filters: SearchFilters::default(),
            facets: None,
            search_results: SearchResults::default(),
            home: Home::default(),
            page: Page::default(),
            catalog_request: None,
            artwork: artwork::Cache::default(),
            brand_texture: None,
            #[cfg(target_arch = "wasm32")]
            mesh_code: crate::mesh::launch_code(),
            #[cfg(target_arch = "wasm32")]
            mesh_name: String::new(),
            #[cfg(target_arch = "wasm32")]
            mesh_step: None,
            #[cfg(target_arch = "wasm32")]
            mesh_invite: None,
            #[cfg(target_arch = "wasm32")]
            mesh_invite_check: 0,
            previous_shelves: Vec::new(),
            error: String::new(),
            tx,
            rx,
            pending: None,
            auth_drains: Arc::new(AtomicUsize::new(0)),
            discovery: None,
            form: None,
            selection: vec![],
            confirmed: false,
            peer_id: String::new(),
            peer_address: String::new(),
            playback: None,
            retired: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            video: None,
            #[cfg(not(target_arch = "wasm32"))]
            video_session: None,
        }
    }
}

fn web_hosted_server() -> String {
    #[cfg(target_arch = "wasm32")]
    if let Some(url) = web_sys::window()
        .and_then(|window| window.location().href().ok())
        .and_then(|page| protocol::hosted_server(&page))
    {
        return url.to_string();
    }
    String::new()
}

fn paint_art(ui: &egui::Ui, texture: &egui::TextureHandle, rect: egui::Rect) {
    let source = texture.size_vec2();
    let source_ratio = source.x / source.y;
    let target_ratio = rect.width() / rect.height();
    let uv = if source_ratio > target_ratio {
        let fraction = target_ratio / source_ratio;
        egui::Rect::from_min_max(
            egui::pos2((1.0 - fraction) / 2.0, 0.0),
            egui::pos2((1.0 + fraction) / 2.0, 1.0),
        )
    } else {
        let fraction = source_ratio / target_ratio;
        egui::Rect::from_min_max(
            egui::pos2(0.0, (1.0 - fraction) / 2.0),
            egui::pos2(1.0, (1.0 + fraction) / 2.0),
        )
    };
    ui.painter()
        .image(texture.id(), rect, uv, egui::Color32::WHITE);
}
pub fn configure(ctx: &egui::Context) {
    theme::configure(ctx);
}

impl Bluefin {
    fn brand(&mut self, ui: &mut egui::Ui) {
        if self.brand_texture.is_none() {
            if let Ok(image) = image::load_from_memory(include_bytes!("../assets/bluefin-mark.png"))
            {
                let image = image.into_rgba8();
                self.brand_texture = Some(ui.ctx().load_texture(
                    "bluefin-mark",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [image.width() as usize, image.height() as usize],
                        image.as_raw(),
                    ),
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
        ui.horizontal(|ui| {
            if let Some(texture) = &self.brand_texture {
                ui.add(egui::Image::new(texture).fit_to_exact_size(egui::vec2(36.0, 36.0)));
            }
            ui.label(
                egui::RichText::new("Bluefin")
                    .size(24.0)
                    .strong()
                    .color(ui.visuals().hyperlink_color),
            );
        });
    }
    fn section_title(section: Section) -> &'static str {
        section.title()
    }
    fn open_section(&mut self, ctx: &egui::Context, section: Section) {
        self.model.section = section;
        self.query.clear();
        self.previous_shelves.clear();
        self.discovery = None;
        if section == Section::Search {
            self.filters = SearchFilters::default();
            self.search_results = SearchResults::default();
        }
        self.browse(ctx, None);
    }
    fn navigation(&mut self, ui: &mut egui::Ui, compact: bool) {
        for (name, sections) in [
            ("Categories", Section::PRIMARY.as_slice()),
            ("Library tools", Section::UTILITIES.as_slice()),
        ] {
            if !compact && name == "Library tools" {
                ui.add_space(16.0);
                ui.small(name);
            }
            let mut entries = |ui: &mut egui::Ui| {
                for section in sections {
                    let response = ui.add_sized(
                        [if compact { 94.0 } else { ui.available_width() }, 44.0],
                        egui::Button::new(section.title()).selected(self.model.section == *section),
                    );
                    if !compact && response.gained_focus() {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    if response.clicked() {
                        self.open_section(&ui.ctx().clone(), *section);
                    }
                }
            };
            if compact {
                ui.horizontal_wrapped(entries);
            } else {
                entries(ui);
            }
        }
    }
    fn open_detail(&mut self, ctx: &egui::Context, mut item: Item) {
        if self.model.fixture {
            self.model.detail = Some(item);
            return;
        }
        let Some(session) = self.model.session.clone() else {
            return;
        };
        let selected = item.id;
        if matches!(item.kind.as_str(), "Program" | "LiveTvProgram") {
            let Some(channel) = item.channel_id.filter(|id| !id.is_nil()) else {
                self.model.detail = Some(item);
                return;
            };
            item.id = channel;
            item.kind = "TvChannel".into();
        }
        let history = (self.model.section == Section::Search)
            .then_some(self.home.history_generation)
            .flatten();
        let query = self.query.clone();
        self.launch(ctx, async move {
            let path = format!("Users/{}/Items/{}", session.user, item.id);
            let fetch = protocol::request(&session.base, &session.token, "GET", &path, &[], None);
            let signal = async {
                if let Some(generation) = history {
                    catalog::record_search(&session, generation, &query, Some(selected)).await;
                }
            };
            let (value, ()) = futures_util::join!(fetch, signal);
            value.map(|value| Outcome::Data("detail", value))
        });
    }
    fn catalog_item(&mut self, ui: &mut egui::Ui, item: &Item, tile: bool) {
        let width = ui.available_width();
        let height = if tile {
            (width * 1.35).clamp(180.0, 300.0)
        } else {
            100.0
        };
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
        if ui.is_rect_visible(rect) {
            self.artwork.request(item.id, false);
        }
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &item.name));
        ui.painter()
            .rect_filled(rect, 10.0, ui.visuals().faint_bg_color);
        if let Some(texture) = self.artwork.texture(item.id, false) {
            let target = if tile {
                rect
            } else {
                egui::Rect::from_min_size(rect.min, egui::vec2(70.0, 100.0))
            };
            paint_art(ui, &texture, target);
        }
        let footer = if tile {
            egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 76.0), rect.max)
        } else {
            egui::Rect::from_min_max(rect.min + egui::vec2(80.0, 0.0), rect.max)
        };
        ui.painter().rect_filled(
            footer,
            if tile { 8.0 } else { 0.0 },
            egui::Color32::from_rgba_unmultiplied(6, 23, 39, 235),
        );
        let name = ui.painter().layout(
            item.name.clone(),
            egui::FontId::proportional(16.0),
            egui::Color32::from_rgb(232, 237, 237),
            footer.width() - 20.0,
        );
        ui.painter().with_clip_rect(footer.shrink(8.0)).galley(
            footer.min + egui::vec2(10.0, 8.0),
            name,
            egui::Color32::WHITE,
        );
        let meta = if self.model.section == Section::Guide {
            item.channel_name.as_deref().unwrap_or(&item.kind)
        } else {
            &item.kind
        };
        ui.painter().text(
            egui::pos2(footer.left() + 10.0, footer.bottom() - 17.0),
            egui::Align2::LEFT_CENTER,
            meta,
            egui::FontId::proportional(12.0),
            egui::Color32::from_rgb(174, 188, 199),
        );
        if item.progress() > 0.0 {
            let track = egui::Rect::from_min_size(
                egui::pos2(rect.left(), rect.bottom() - 4.0),
                egui::vec2(rect.width() * item.progress(), 4.0),
            );
            ui.painter()
                .rect_filled(track, 2.0, egui::Color32::from_rgb(169, 162, 201));
        }
        if response.hovered() || response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                10.0,
                egui::Stroke::new(2.0_f32, ui.visuals().hyperlink_color),
                egui::StrokeKind::Inside,
            );
        }
        if response.clicked() {
            self.open_detail(&ui.ctx().clone(), item.clone());
        }
    }
    fn media_grid(&mut self, ui: &mut egui::Ui, rows: &[Item]) {
        let tiled = ui.available_width() >= 520.0 && self.model.section != Section::Guide;
        if tiled {
            let columns = ((ui.available_width() / 200.0) as usize).clamp(2, 5);
            for row in rows.chunks(columns) {
                ui.columns(columns, |uis| {
                    for (item, column) in row.iter().zip(uis) {
                        self.catalog_item(column, item, true);
                    }
                });
                ui.add_space(12.0);
            }
        } else {
            for item in rows {
                self.catalog_item(ui, item, false);
                ui.add_space(8.0);
            }
        }
    }
    fn featured(&mut self, ui: &mut egui::Ui, item: &Item) {
        self.artwork.request(item.id, true);
        let width = ui.available_width();
        let height = (width * 0.46).clamp(260.0, 390.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, 14.0, egui::Color32::from_rgb(6, 23, 39));
        if let Some(texture) = self.artwork.texture(item.id, true) {
            paint_art(ui, &texture, rect);
        }
        let footer =
            egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 160.0), rect.max);
        ui.painter().rect_filled(
            footer,
            10.0,
            egui::Color32::from_rgba_unmultiplied(6, 23, 39, 225),
        );
        let content = footer.shrink(18.0);
        let title = ui.painter().layout(
            item.name.clone(),
            egui::FontId::proportional(26.0),
            egui::Color32::from_rgb(232, 237, 237),
            content.width(),
        );
        ui.painter()
            .with_clip_rect(egui::Rect::from_min_size(
                content.min,
                egui::vec2(content.width(), 76.0),
            ))
            .galley(content.min, title, egui::Color32::WHITE);
        let button_y = rect.bottom() - 62.0;
        if item.playable()
            && ui
                .put(
                    egui::Rect::from_min_size(
                        egui::pos2(rect.left() + 18.0, button_y),
                        egui::vec2(130.0, 42.0),
                    ),
                    theme::primary_button(
                        ui,
                        if item.progress() > 0.0 {
                            "Continue"
                        } else {
                            "Play"
                        },
                    ),
                )
                .clicked()
        {
            self.play(item.clone());
        }
        let offset = if item.playable() { 160.0 } else { 18.0 };
        if ui
            .put(
                egui::Rect::from_min_size(
                    egui::pos2(rect.left() + offset, button_y),
                    egui::vec2(110.0, 42.0),
                ),
                egui::Button::new("Details"),
            )
            .clicked()
        {
            self.open_detail(&ui.ctx().clone(), item.clone());
        }
    }
    fn search_controls(&mut self, ui: &mut egui::Ui) {
        let before = self.filters.clone();
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_label("Media type")
                .selected_text(self.filters.scope.title())
                .show_ui(ui, |ui| {
                    for scope in SearchScope::ALL {
                        ui.selectable_value(&mut self.filters.scope, scope, scope.title());
                    }
                });
            if let Some(facets) = &self.facets {
                let studio = facets
                    .studios
                    .iter()
                    .find(|item| Some(item.id) == self.filters.studio)
                    .map_or("All studios", |item| item.name.as_str());
                egui::ComboBox::from_label("Studio")
                    .selected_text(studio)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.filters.studio, None, "All studios");
                        for item in &facets.studios {
                            ui.selectable_value(
                                &mut self.filters.studio,
                                Some(item.id),
                                &item.name,
                            );
                        }
                    });
                egui::ComboBox::from_label("Tag")
                    .selected_text(if self.filters.tag.is_empty() {
                        "All tags"
                    } else {
                        &self.filters.tag
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.filters.tag, String::new(), "All tags");
                        for tag in &facets.tags {
                            ui.selectable_value(&mut self.filters.tag, tag.clone(), tag);
                        }
                    });
                let collection = facets
                    .collections
                    .iter()
                    .find(|item| Some(item.id) == self.filters.collection)
                    .map_or("All collections", |item| item.name.as_str());
                egui::ComboBox::from_label("Collection")
                    .selected_text(collection)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.filters.collection, None, "All collections");
                        for item in &facets.collections {
                            ui.selectable_value(
                                &mut self.filters.collection,
                                Some(item.id),
                                &item.name,
                            );
                        }
                    });
            }
            if ui.button("Clear refinements").clicked() {
                self.filters = SearchFilters::default();
            }
        });
        ui.small("Studios, service tags and custom collections come from your server.");
        if let Some(facets) = &self.facets {
            if facets.truncated {
                ui.small("Showing a limited set of refinement choices. Search still covers your full account.");
            }
            if !facets.warning.is_empty() {
                ui.label(&facets.warning);
            }
        }
        if self.filters != before {
            self.run_search(&ui.ctx().clone(), false);
        }
    }
    fn run_search(&mut self, ctx: &egui::Context, record: bool) {
        self.model.detail = None;
        self.previous_shelves.clear();
        self.model.rows.clear();
        self.search_results = SearchResults::default();
        let Some(session) = self.model.session.clone() else {
            return;
        };
        let filters = self.filters.clone();
        let query = self.query.clone();
        let needs_facets = self.facets.is_none();
        let generation = self.home.history_generation.filter(|_| record);
        self.launch(ctx, async move {
            let choices = async {
                if needs_facets {
                    catalog::facets(&session).await.map(Some)
                } else {
                    Ok(None)
                }
            };
            let signal = async {
                if let Some(generation) = generation {
                    catalog::record_search(&session, generation, &query, None).await;
                }
            };
            let (results, facets, ()) =
                futures_util::join!(catalog::search(&session, &query, &filters), choices, signal);
            Ok(Outcome::Search(results?, facets?))
        });
    }
    fn load_more(&mut self, ctx: &egui::Context, lane: Option<usize>) {
        if self.pending.is_some() {
            return;
        }
        let Some(session) = self.model.session.clone() else {
            return;
        };
        if let Some(index) = lane {
            let Some(shelf) = self
                .search_results
                .shelves
                .get(index)
                .filter(|shelf| shelf.page.more())
            else {
                return;
            };
            let request = shelf.request.clone();
            let start = shelf.page.next;
            self.launch(ctx, async move {
                catalog::page(&session, &request, start)
                    .await
                    .map(|page| Outcome::SearchMore(index, page))
            });
        } else if self.page.more()
            && let Some(request) = self.catalog_request.clone()
        {
            let start = self.page.next;
            self.launch(ctx, async move {
                catalog::page(&session, &request, start)
                    .await
                    .map(|page| Outcome::Rows(page, true))
            });
        }
    }
    fn cancel(&mut self) {
        if let Some(handle) = self.pending.take() {
            handle.abort();
        }
        self.model.epoch = self.model.epoch.wrapping_add(1);
    }
    fn reset(&mut self) {
        let old_session = self.model.session.clone();
        self.cancel();
        self.model.clear();
        self.artwork.clear();
        self.home = Home::default();
        self.filters = SearchFilters::default();
        self.facets = None;
        self.search_results = SearchResults::default();
        self.page = Page::default();
        self.catalog_request = None;
        self.previous_shelves.clear();
        self.password.clear();
        self.connected_server = None;
        self.device_sign_in = None;
        #[cfg(target_arch = "wasm32")]
        {
            self.mesh_code.clear();
            self.mesh_name.clear();
            self.mesh_step = None;
            self.mesh_invite = None;
        }
        self.query.clear();
        self.discovery = None;
        self.form = None;
        self.selection.clear();
        self.confirmed = false;
        self.error.clear();
        self.peer_id.clear();
        self.peer_address.clear();
        if let Some(service) = self.playback.take() {
            service.send(PlaybackCommand::Logout);
            self.retired.push(service);
        } else if let Some(session) = old_session {
            let service = Service::start(session, false, None);
            service.send(PlaybackCommand::Logout);
            self.retired.push(service);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.video = None;
        }
    }
    fn launch(&mut self, ctx: &egui::Context, future: impl Work) {
        self.launch_inner(ctx, future, false);
    }
    fn connect_server(&mut self, ctx: &egui::Context) {
        let address = self.server.trim();
        let preferred = if address.contains("://") {
            address.to_owned()
        } else {
            format!("https://{address}")
        };
        match protocol::server(&preferred) {
            Ok(base) => self.launch(ctx, async move {
                protocol::connect(base).await.map(Outcome::Connected)
            }),
            Err(error) => self.error = error,
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn mesh_result(progress: crate::mesh::Progress) -> Outcome {
        match progress {
            crate::mesh::Progress::Empty => Outcome::MeshEmpty,
            crate::mesh::Progress::Pairing(step) => Outcome::MeshStep(step),
            crate::mesh::Progress::Connected(session) => Outcome::Login(session),
        }
    }
    #[cfg(target_arch = "wasm32")]
    fn begin_mesh(&mut self, ctx: &egui::Context) {
        let code = self.mesh_code.clone();
        let name = self.mesh_name.clone();
        self.launch_sign_in(ctx, async move {
            crate::mesh::begin(code, name, false)
                .await
                .map(Self::mesh_result)
        });
    }
    #[cfg(target_arch = "wasm32")]
    fn resume_mesh(&mut self, ctx: &egui::Context) {
        self.launch_sign_in(ctx, async {
            crate::mesh::resume().await.map(Self::mesh_result)
        });
    }
    #[cfg(target_arch = "wasm32")]
    fn proceed_mesh(&mut self, ctx: &egui::Context) {
        self.launch_sign_in(ctx, async {
            crate::mesh::proceed().await.map(Self::mesh_result)
        });
    }
    #[cfg(target_arch = "wasm32")]
    fn issue_mesh_invite(&mut self, ctx: &egui::Context) {
        self.mesh_invite = None;
        self.call(
            ctx,
            "meshInvite",
            "POST",
            "Bluefin/Accounts/Mesh/Invite".into(),
            vec![],
            Some(serde_json::json!({"intent":"login","lifetimeSeconds":600})),
        );
    }
    #[cfg(target_arch = "wasm32")]
    fn copy_mesh_invite(&mut self, ctx: &egui::Context, link: bool) {
        let Some(invite) = self.mesh_invite.as_ref().filter(|invite| invite.current()) else {
            return;
        };
        let value = if link {
            invite.link.clone()
        } else {
            invite.code.clone()
        };
        self.launch(ctx, async move {
            crate::mesh::copy_code(&value)
                .await
                .map_err(|_| "Copy was not allowed. Select and copy the code below.".to_string())?;
            Ok(Outcome::Data("meshCopied", Value::Null))
        });
    }
    #[cfg(target_arch = "wasm32")]
    fn import_mesh_image(&mut self, ctx: &egui::Context, file: web_sys::File) {
        let tx = self.tx.clone();
        let epoch = self.model.epoch;
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = if file.size() > 8.0 * 1024.0 * 1024.0 {
                Err("Choose a QR image smaller than 8 MiB.".into())
            } else {
                match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                    Ok(value) => crate::mesh::image_code(&js_sys::Uint8Array::new(&value).to_vec())
                        .map(|code| Outcome::Data("meshImage", Value::String(code))),
                    Err(_) => Err("The QR image could not be read.".into()),
                }
            };
            let _ = tx.send((epoch, result));
            ctx.request_repaint();
        });
    }

    fn submit_account(&mut self, ctx: &egui::Context, create: bool) {
        let Some(server) = &self.connected_server else {
            return;
        };
        if create && (!server.options.allow_signups || self.password.chars().count() < 15) {
            return;
        }
        let base = server.base.clone();
        let name = self.name.trim().to_owned();
        let password = std::mem::take(&mut self.password);
        self.launch_sign_in(ctx, async move {
            if create {
                protocol::signup(base, name, password)
                    .await
                    .map(Outcome::Login)
            } else {
                protocol::login(base, name, password)
                    .await
                    .map(Outcome::Login)
            }
        });
    }

    fn begin_device_sign_in(&mut self, ctx: &egui::Context, passkey: bool) {
        let Some(server) = &self.connected_server else {
            return;
        };
        if (passkey && !server.options.passkeys) || (!passkey && !server.options.quick_connect) {
            return;
        }
        let base = server.base.clone();
        self.launch(ctx, async move {
            protocol::begin_device_sign_in(base, passkey)
                .await
                .map(Outcome::DeviceSignIn)
        });
    }

    fn finish_device_sign_in(&mut self, ctx: &egui::Context) {
        if let Some(request) = self.device_sign_in.clone() {
            self.launch_sign_in(ctx, async move {
                request
                    .finish()
                    .await
                    .map(|session| session.map_or(Outcome::Waiting, Outcome::Login))
            });
        }
    }

    fn cancel_device_sign_in(&mut self, ctx: &egui::Context) {
        if let Some(request) = self.device_sign_in.take() {
            self.launch(ctx, async move {
                request
                    .cancel()
                    .await
                    .map(|()| Outcome::Data("accountCancel", Value::Null))
            });
        }
    }

    fn launch_sign_in(&mut self, ctx: &egui::Context, future: impl Work) {
        self.launch_inner(ctx, future, true);
    }
    fn launch_inner(&mut self, ctx: &egui::Context, future: impl Work, drain: bool) {
        self.cancel();
        self.error.clear();
        let epoch = self.model.epoch;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let (handle, registration) = AbortHandle::new_pair();
        self.pending = Some(handle);
        let drain_guard = drain.then(|| {
            self.auth_drains.fetch_add(1, Ordering::Relaxed);
            SignInDrain(self.auth_drains.clone())
        });
        let task = async move {
            let _drain_guard = drain_guard;
            let value = if drain {
                Some(future.await)
            } else {
                Abortable::new(future, registration).await.ok()
            };
            if let Some(value) = value {
                let _ = tx.send((epoch, value));
                ctx.request_repaint();
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            static RT: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
            RT.get_or_init(|| tokio::runtime::Runtime::new().unwrap())
                .spawn(task);
        }
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(task);
    }
    fn call(
        &mut self,
        ctx: &egui::Context,
        tag: &'static str,
        method: &str,
        path: String,
        query: Vec<(&'static str, String)>,
        body: Option<Value>,
    ) {
        let Some(session) = self.model.session.clone() else {
            return;
        };
        let method = method.to_owned();
        self.launch(ctx, async move {
            protocol::request(&session.base, &session.token, &method, &path, &query, body)
                .await
                .map(|v| Outcome::Data(tag, v))
        });
    }
    fn back_to_shelf(&mut self) {
        if self.model.detail.take().is_some() {
            self.cancel();
        } else if let Some(previous) = self.previous_shelves.pop() {
            self.cancel();
            self.model.rows = previous.rows;
            self.model.detail = previous.detail;
            self.page = previous.page;
            self.catalog_request = previous.request;
        }
    }

    fn browse(&mut self, ctx: &egui::Context, parent: Option<Item>) {
        self.cancel();
        self.artwork.cancel_pending();
        if parent.is_some() {
            if self.previous_shelves.len() == 16 {
                self.previous_shelves.remove(0);
            }
            self.previous_shelves.push(PreviousShelf {
                rows: std::mem::take(&mut self.model.rows),
                detail: self.model.detail.take(),
                page: std::mem::take(&mut self.page),
                request: self.catalog_request.take(),
            });
        } else {
            self.previous_shelves.clear();
            self.model.detail = None;
            self.model.rows.clear();
            self.page = Page::default();
            self.catalog_request = None;
        }
        if self.model.fixture {
            self.model.rows = if parent.is_some() {
                vec![Item {
                    id: uuid::Uuid::from_u128(4),
                    name: "First light".into(),
                    kind: "Episode".into(),
                    overview: "Original episode fixture. No film file supplied.".into(),
                    start_date: None,
                    end_date: None,
                    channel_name: None,
                    channel_id: None,
                    ..Default::default()
                }]
            } else {
                self.model.fixture_rows()
            };
            return;
        }
        if self.model.section == Section::Queue {
            if self.model.queue_dirty {
                return;
            }
            self.call(
                ctx,
                "queue",
                "GET",
                "Bluefin/Music/Queue".into(),
                vec![],
                None,
            );
            return;
        }
        if self.model.section == Section::Discovery {
            if !self
                .model
                .capabilities
                .as_ref()
                .and_then(|c| c.discovery.as_ref())
                .is_some_and(|c| c.search)
            {
                self.error = "Server discovery is unavailable for this account".into();
                return;
            }
            self.call(
                ctx,
                "discovery",
                "GET",
                "Bluefin/Discovery/Search".into(),
                vec![("query", self.query.clone()), ("page", "1".into())],
                None,
            );
            return;
        }
        let Some(session) = self.model.session.clone() else {
            return;
        };
        if parent.is_none() {
            match self.model.section {
                Section::Home => {
                    self.launch(ctx, async move {
                        catalog::home(&session).await.map(Outcome::Home)
                    });
                    return;
                }
                Section::Search => {
                    self.run_search(ctx, false);
                    return;
                }
                Section::Account => return,
                Section::Sports => {
                    self.launch(ctx, async move {
                        catalog::sports(&session)
                            .await
                            .map(|page| Outcome::Rows(page, false))
                    });
                    return;
                }
                _ => {}
            }
        }
        let request = if let Some(item) = parent {
            Request::children(session.user, &item)
        } else if self.model.section == Section::Guide {
            if !self
                .model
                .capabilities
                .as_ref()
                .is_some_and(|caps| caps.viewer.live_tv_access)
            {
                self.error = "Live TV is not permitted for this account".into();
                return;
            }
            Request {
                path: "LiveTv/Programs".into(),
                query: vec![
                    ("UserId", session.user.to_string()),
                    ("Fields", "Overview,RunTimeTicks".into()),
                ],
            }
        } else if self.model.section == Section::Collections {
            Request {
                path: format!("Users/{}/Views", session.user),
                query: vec![],
            }
        } else {
            Request::items(session.user, self.model.section.kind(), None)
        };
        self.catalog_request = Some(request.clone());
        self.launch(ctx, async move {
            catalog::page(&session, &request, 0)
                .await
                .map(|page| Outcome::Rows(page, false))
        });
    }

    fn apply(&mut self, tag: &str, value: Value) -> Result<(), String> {
        match tag {
            "accountCancel" => {}
            "historyClear" => {
                let generation = value["generation"]
                    .as_u64()
                    .ok_or("Invalid history revision")?;
                self.home.history_generation = Some(generation);
                self.home.shelves.retain(|shelf| shelf.reason != "search");
                self.home.featured = self
                    .home
                    .shelves
                    .iter()
                    .find_map(|shelf| shelf.items.first())
                    .cloned();
            }
            "capabilities" => {
                let c: Capabilities =
                    serde_json::from_value(value).map_err(|_| "Invalid capabilities")?;
                self.model.admit(c)?;
            }
            "rows" => self.model.rows = protocol::items(value)?,
            "detail" => {
                self.model.detail =
                    Some(serde_json::from_value(value).map_err(|_| "Invalid detail")?)
            }
            "queue" => {
                let q: MusicQueue = serde_json::from_value(value).map_err(|_| "Invalid queue")?;
                self.model.replace_queue(q)?;
            }
            "discovery" => {
                let page: bluefin_contract::discovery::DiscoveryPage =
                    serde_json::from_value(value).map_err(|_| "Invalid discovery page")?;
                if page.results.len() > 100 {
                    return Err("Discovery page exceeds bounds".into());
                }
                self.discovery = Some(page);
            }
            "form" => {
                let form: bluefin_contract::discovery::ActionForm =
                    serde_json::from_value(value).map_err(|_| "Invalid action form")?;
                if form.choices.len() > 256 {
                    return Err("Too many choices".into());
                }
                self.form = Some(form);
                self.selection.clear();
                self.confirmed = false;
            }
            "receipt" => {
                let receipt: bluefin_contract::discovery::ActionReceipt =
                    serde_json::from_value(value).map_err(|_| "Invalid receipt")?;
                self.error = format!("{}: {}", receipt.status, receipt.message);
                self.form = None;
            }
            _ => {}
        }
        Ok(())
    }
    fn play(&mut self, mut item: Item) {
        if matches!(item.kind.as_str(), "Program" | "LiveTvProgram")
            && let Some(channel) = item.channel_id
        {
            item.id = channel;
            item.kind = "TvChannel".into();
        }
        let mut selection = Selection::default();
        if let Some(data) = &item.user_data {
            selection.position = data.playback_position_ticks as f64 / 10_000_000.0;
        }
        self.play_selected(item, selection, None);
    }
    fn play_selected(&mut self, item: Item, selection: Selection, queue_index: Option<usize>) {
        if !self.model.fixture
            && !self.model.capabilities.as_ref().is_some_and(|c| {
                c.viewer.media_playback && (item.kind != "TvChannel" || c.viewer.live_tv_access)
            })
        {
            self.error = "Playback is not permitted for this account".into();
            return;
        }
        if self.playback.is_none() {
            let Some(session) = self.model.session.clone() else {
                self.error = "Sign in to the synthetic fixture server or your server first".into();
                return;
            };
            let peer = if self.peer_id.is_empty() && self.peer_address.is_empty() {
                None
            } else {
                match self.peer_address.parse() {
                    Ok(address) => Some((self.peer_id.clone(), address)),
                    Err(_) => {
                        self.error = "Enter a verified numeric Iroh address".into();
                        return;
                    }
                }
            };
            self.playback = Some(Service::start(session, self.model.fixture, peer));
        }
        let queue = queue_index.map(|index| (self.model.queue.queue.clone(), index));
        self.playback.as_ref().unwrap().send(PlaybackCommand::Play(
            Box::new(item),
            selection,
            queue,
        ));
    }
    fn poll(&mut self, ctx: &egui::Context) {
        #[cfg(target_arch = "wasm32")]
        if let Some(invite) = &self.mesh_invite {
            if !invite.current() {
                self.mesh_invite = None;
            } else if self.pending.is_none()
                && mesh_connect::browser::now_ms().unwrap_or(0) >= self.mesh_invite_check
            {
                let handle = invite.handle.clone();
                self.mesh_invite_check = mesh_connect::browser::now_ms()
                    .unwrap_or(0)
                    .saturating_add(3000);
                self.call(
                    ctx,
                    "meshInviteStatus",
                    "GET",
                    format!("Bluefin/Accounts/Mesh/Invites/{handle}"),
                    vec![],
                    None,
                );
            }
        }
        while let Ok((epoch, result)) = self.rx.try_recv() {
            if !self.model.accepts(epoch) {
                if let Ok(Outcome::Login(session)) = result {
                    self.retired.push(Service::revoke_only(session));
                }
                continue;
            }
            self.pending = None;
            match result {
                #[cfg(target_arch = "wasm32")]
                Ok(Outcome::MeshStep(step)) => {
                    self.mesh_step = Some(step);
                    self.mesh_code.clear();
                    self.connected_server = None;
                }
                #[cfg(target_arch = "wasm32")]
                Ok(Outcome::MeshEmpty) => {
                    self.mesh_step = None;
                    self.error = "No saved browser connection.".into();
                }
                Ok(Outcome::Connected(server)) => self.connected_server = Some(server),
                Ok(Outcome::DeviceSignIn(request)) => self.device_sign_in = Some(request),
                Ok(Outcome::Waiting) => self.error = "Finish approving the request first.".into(),
                Ok(Outcome::Login(s)) => {
                    self.device_sign_in = None;
                    #[cfg(target_arch = "wasm32")]
                    {
                        self.mesh_step = None;
                        self.mesh_code.clear();
                    }
                    self.model.session = Some(s);
                    self.call(
                        ctx,
                        "capabilities",
                        "GET",
                        "Bluefin/Capabilities".into(),
                        vec![],
                        None,
                    );
                }
                #[cfg(target_arch = "wasm32")]
                Ok(Outcome::Data("meshInvite", value)) => {
                    match crate::mesh::IssuedInvite::from_response(value) {
                        Ok(invite) => {
                            self.mesh_invite = Some(invite);
                            self.mesh_invite_check = mesh_connect::browser::now_ms()
                                .unwrap_or(0)
                                .saturating_add(3000);
                        }
                        Err(error) => self.error = error,
                    }
                }
                #[cfg(target_arch = "wasm32")]
                Ok(Outcome::Data("meshInviteStatus", value)) => {
                    if value["state"] != "issued" {
                        self.mesh_invite = None;
                    }
                    self.mesh_invite_check = mesh_connect::browser::now_ms()
                        .unwrap_or(0)
                        .saturating_add(3000);
                }
                #[cfg(target_arch = "wasm32")]
                Ok(Outcome::Data("meshCopied", _)) => self.error = "Copied.".into(),
                #[cfg(target_arch = "wasm32")]
                Ok(Outcome::Data("meshImage", Value::String(code))) => {
                    self.mesh_code = code;
                    self.error = "QR image read. Continue to connect.".into();
                }
                Ok(Outcome::Home(home)) => {
                    self.model.rows = home
                        .shelves
                        .iter()
                        .flat_map(|shelf| shelf.items.clone())
                        .collect();
                    self.error = home.warning.clone();
                    self.home = home;
                }
                Ok(Outcome::Rows(page, append)) => {
                    if append {
                        self.page.append(page);
                    } else {
                        self.page = page;
                    }
                    self.model.rows = self.page.items.clone();
                }
                Ok(Outcome::Search(results, facets)) => {
                    self.model.rows = results
                        .shelves
                        .iter()
                        .flat_map(|shelf| shelf.page.items.clone())
                        .collect();
                    self.search_results = results;
                    if let Some(facets) = facets {
                        self.facets = Some(facets);
                    }
                }
                Ok(Outcome::SearchMore(index, page)) => {
                    if let Some(shelf) = self.search_results.shelves.get_mut(index) {
                        shelf.page.append(page);
                    }
                    self.model.rows = self
                        .search_results
                        .shelves
                        .iter()
                        .flat_map(|shelf| shelf.page.items.clone())
                        .collect();
                }
                Ok(Outcome::Data(tag, v)) => match self.apply(tag, v) {
                    Err(e) => self.error = e,
                    Ok(()) => {
                        if tag == "capabilities" {
                            if self
                                .model
                                .capabilities
                                .as_ref()
                                .is_some_and(|c| c.music_queue_sync)
                            {
                                self.call(
                                    ctx,
                                    "queue",
                                    "GET",
                                    "Bluefin/Music/Queue".into(),
                                    vec![],
                                    None,
                                );
                            } else {
                                self.browse(ctx, None);
                            }
                        } else if tag == "queue"
                            && self.model.section != Section::Queue
                            && self.model.rows.is_empty()
                        {
                            self.browse(ctx, None);
                        }
                    }
                },
                Err(e) => {
                    if e.starts_with("Account access denied") {
                        self.reset();
                    }
                    self.error = e;
                }
            }
        }
        if self.model.session.is_some() && !self.model.fixture {
            if self.model.section == Section::Home
                && let Some(item) = &self.home.featured
            {
                self.artwork.request(item.id, true);
            }
            #[cfg(target_arch = "wasm32")]
            self.artwork.request_visible();
            self.artwork.poll(ctx, self.model.session.as_ref());
        }
    }
    fn playback_ui(&mut self, ui: &mut egui::Ui) {
        let Some(service) = &self.playback else {
            return;
        };
        let status = service.snapshot();
        ui.small("NOW PLAYING");
        if status.busy {
            ui.spinner();
        }
        ui.label(&status.message);
        if let Some(plan) = status.plan {
            ui.horizontal_wrapped(|ui| {
                ui.heading(&plan.item.name);
                ui.label(if plan.live {
                    "Live channel"
                } else {
                    &plan.method
                });
            });
            #[cfg(not(target_arch = "wasm32"))]
            {
                if self.video_session.as_ref() != Some(&plan.play_session) {
                    self.video = None;
                    self.video_session = Some(plan.play_session.clone());
                }
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(16));
                if let Some(d) = service.diagnostics() {
                    ui.small(format!(
                        "{}×{} · {} · {} Hz / {} ch · {} · decoder: {} · source transfer: {}",
                        d.source_size[0],
                        d.source_size[1],
                        d.codec,
                        d.audio_rate,
                        d.audio_channels,
                        d.render_path,
                        if d.hwdec.is_empty() {
                            "unreported"
                        } else {
                            &d.hwdec
                        },
                        d.source_transfer
                    ));
                }
                if crate::mpv::gl_active() {
                    self.video = None;
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 300.0),
                        egui::Sense::hover(),
                    );
                    crate::gl_video::callback(ui, rect, true);
                }
                if let Some(frame) = service.frame() {
                    let image = egui::ColorImage::from_rgba_unmultiplied(frame.size, &frame.rgba);
                    if let Some(texture) = &mut self.video {
                        texture.set(image, egui::TextureOptions::LINEAR);
                    } else {
                        self.video = Some(ui.ctx().load_texture(
                            "decoded-video",
                            image,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                }
                if let Some(texture) = &self.video {
                    ui.add(
                        egui::Image::new(texture)
                            .max_width(ui.available_width())
                            .max_height(300.0),
                    );
                }
            }
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(if status.paused { "Resume" } else { "Pause" })
                    .clicked()
                {
                    service.send(PlaybackCommand::Pause(!status.paused));
                }
                if ui.button("Stop playback").clicked() {
                    service.send(PlaybackCommand::Stop);
                }
                ui.label(format!("{:.1} / {:.1} s", status.position, status.duration));
            });
            if !plan.live && status.duration > 0.0 {
                let mut position = status.position;
                if ui
                    .add(
                        egui::Slider::new(&mut position, 0.0..=status.duration)
                            .text("Seek seconds"),
                    )
                    .drag_stopped()
                {
                    service.send(PlaybackCommand::Seek(position));
                }
            }
            let mut selection = plan.selection;
            selection.position = status.position;
            let mut changed = false;
            ui.small("Stream selection · negotiated by server");
            ui.horizontal_wrapped(|ui| {
                egui::ComboBox::from_label("Audio stream")
                    .selected_text(
                        selection
                            .audio
                            .map_or("Server default".into(), |n| n.to_string()),
                    )
                    .show_ui(ui, |ui| {
                        for track in plan.tracks.iter().filter(|s| s.kind == "Audio") {
                            changed |= ui
                                .selectable_value(
                                    &mut selection.audio,
                                    Some(track.index),
                                    format!("{} {} {}", track.index, track.language, track.codec),
                                )
                                .changed();
                        }
                    });
                egui::ComboBox::from_label("Subtitles")
                    .selected_text(selection.subtitle.map_or("Server default".into(), |n| {
                        if n < 0 { "Off".into() } else { n.to_string() }
                    }))
                    .show_ui(ui, |ui| {
                        changed |= ui
                            .selectable_value(&mut selection.subtitle, Some(-1), "Off")
                            .changed();
                        for track in plan.tracks.iter().filter(|s| s.kind == "Subtitle") {
                            changed |= ui
                                .selectable_value(
                                    &mut selection.subtitle,
                                    Some(track.index),
                                    format!("{} {} {}", track.index, track.language, track.codec),
                                )
                                .changed();
                        }
                    });
            });
            if changed {
                service.send(PlaybackCommand::Play(Box::new(plan.item), selection, None));
            }
        }
    }
}
impl eframe::App for Bluefin {
    #[cfg(not(target_arch = "wasm32"))]
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        crate::gl_video::shutdown();
    }
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if !self.styled {
            configure(ctx);
            self.styled = true;
        }
        self.poll(ctx);
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
        let connected = self.model.session.is_some() || self.model.fixture;
        let wide = ctx.content_rect().width() >= 760.0;
        egui::TopBottomPanel::top("toolbar")
            .frame(
                egui::Frame::new()
                    .fill(ctx.style().visuals.window_fill)
                    .inner_margin(16),
            )
            .show(ctx, |ui| {
                if !wide {
                    self.brand(ui);
                }
                ui.horizontal_wrapped(|ui| {
                    if wide {
                        self.brand(ui);
                    }
                    ui.small(if self.model.fixture {
                        "Original synthetic fixtures"
                    } else if connected {
                        "Your library"
                    } else {
                        "Connect your library"
                    });
                    if connected && ui.button("Sign out").clicked() {
                        self.reset();
                    }
                    if self.pending.is_some() {
                        ui.spinner();
                        ui.label("Loading…");
                        if ui.button("Cancel").clicked() {
                            self.cancel();
                        }
                    }
                });
            });
        if connected && wide {
            egui::SidePanel::left("library-navigation")
                .exact_width(208.0)
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(ctx.style().visuals.window_fill)
                        .inner_margin(16),
                )
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("library-navigation-scroll")
                        .show(ui, |ui| {
                            ui.add_space(16.0);
                            ui.small("LIBRARY");
                            ui.add_space(12.0);
                            self.navigation(ui, false);
                        });
                });
        }
        if let Some(service) = &self.playback {
            let status = service.snapshot();
            let height = if status.plan.is_none() {
                60.0
            } else if cfg!(target_arch = "wasm32") {
                215.0
            } else {
                500.0_f32.min(ctx.available_rect().height() * 0.75)
            };
            egui::TopBottomPanel::bottom("embedded-player")
                .exact_height(height)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("player-controls")
                        .show(ui, |ui| self.playback_ui(ui));
                });
        }
        egui::CentralPanel::default().frame(egui::Frame::central_panel(&ctx.style()).inner_margin(if wide {24} else {16})).show(ctx,|ui|{
            #[cfg(not(target_arch = "wasm32"))]
            {
                // Also service initialization/retirement with no visible player.
                crate::gl_video::callback(ui,egui::Rect::from_min_size(ui.min_rect().min,egui::vec2(1.0,1.0)),false);
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
            }
            egui::ScrollArea::vertical().id_salt("main-page").show(ui,|ui|{
            if self.model.session.is_none()&&!self.model.fixture{
                if self.auth_drains.load(Ordering::Acquire)>0&&self.pending.is_none(){ui.label("Finishing canceled sign-in cleanup");}
                ui.add_space((ui.available_height() * 0.10).clamp(12.0, 64.0));
                let card_width = ui.available_width().min(440.0);
                ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - card_width) * 0.5).max(0.0));
                ui.vertical(|ui| { ui.set_width(card_width);
                egui::Frame::new().fill(ui.visuals().window_fill).corner_radius(18).inner_margin(24).show(ui, |ui| {
                ui.set_width((card_width - 48.0).max(80.0));
                ui.label(egui::RichText::new("WELCOME TO BLUEFIN").small().color(ui.visuals().hyperlink_color));
                ui.add_space(12.0);
                let ready = self.pending.is_none() && self.auth_drains.load(Ordering::Acquire) == 0
                    && self.retired.iter().all(|service| service.snapshot().finished);
                #[cfg(target_arch = "wasm32")]
                {
                    ui.heading("Connect with an invitation");
                    if let Some(step) = self.mesh_step.clone() {
                        if let Some(url) = &step.external_url { ui.hyperlink_to("Open passkey page", url); }
                        let label = if step.awaiting_approval { "Check approval" } else if step.needs_begin_retry { "Retry saved request" } else { "Verify and finish" };
                        if ui.add_enabled(ready, egui::Button::new(label)).clicked() { self.proceed_mesh(ctx); }
                    } else {
                        ui.label("Paste a connection code or launch link");
                        ui.add(egui::TextEdit::singleline(&mut self.mesh_code).password(true).desired_width(f32::INFINITY));
                        ui.label("Username for a signup invitation");
                        ui.add(egui::TextEdit::singleline(&mut self.mesh_name).desired_width(f32::INFINITY));
                        if ui.add_enabled(ready && !self.mesh_code.is_empty(), theme::primary_button(ui, "Connect")).clicked() { self.begin_mesh(ctx); }
                    }
                    if ui.add_enabled(ready, egui::Button::new("Resume saved connection")).clicked() { self.resume_mesh(ctx); }
                    ui.separator();
                }
                if let Some(request) = &self.device_sign_in {
                    ui.heading(if request.page.is_some() { "Sign in with a passkey" } else { "Quick Connect" });
                    ui.monospace(&request.code);
                    if let Some(page) = &request.page { ui.hyperlink_to("Open passkey page", page.as_str()); }
                    else { ui.label("Enter this code in a client signed in to your server."); }
                    if ui.add_enabled(ready, theme::primary_button(ui, "Finish sign-in")).clicked() { self.finish_device_sign_in(ctx); }
                    if ui.add_enabled(ready, egui::Button::new("Cancel")).clicked() { self.cancel_device_sign_in(ctx); }
                } else if let Some(server) = &self.connected_server {
                    ui.heading(&server.name);
                    let options = server.options.clone();
                    ui.label("Username");
                    ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(f32::INFINITY));
                    ui.label("Password");
                    ui.add(egui::TextEdit::singleline(&mut self.password).password(true).desired_width(f32::INFINITY));
                    let credentials = ready && !self.name.trim().is_empty();
                    if ui.add_enabled(credentials, theme::primary_button(ui, "Sign in")).clicked() { self.submit_account(ctx, false); }
                    if options.passkeys && ui.add_enabled(ready, egui::Button::new("Sign in with a passkey")).clicked() { self.begin_device_sign_in(ctx, true); }
                    if options.quick_connect && ui.add_enabled(ready, egui::Button::new("Quick Connect")).clicked() { self.begin_device_sign_in(ctx, false); }
                    if options.allow_signups && ui.add_enabled(credentials && self.password.chars().count() >= 15, egui::Button::new("Create account")).clicked() { self.submit_account(ctx, true); }
                    if ui.add_enabled(ready, egui::Button::new("Change server")).clicked() { self.connected_server = None; self.password.clear(); }
                } else {
                    ui.heading("Connect to your server");
                    ui.label("Server address");
                    ui.add(egui::TextEdit::singleline(&mut self.server).hint_text("https://media.example.com").desired_width(f32::INFINITY));
                    if ui.add_enabled(ready && !self.server.trim().is_empty(), theme::primary_button(ui, "Continue")).clicked() { self.connect_server(ctx); }
                }
                if ui.add_enabled(self.device_sign_in.is_none()&&self.auth_drains.load(Ordering::Acquire)==0&&self.retired.iter().all(|s|s.snapshot().finished),egui::Button::new("Explore original fixtures")).clicked(){
                    self.reset();self.model.fixture=true;
                    match protocol::fixture_base(){Ok(base)=>self.launch_sign_in(ctx,async move{protocol::login(base,"fixture".into(),String::new()).await.map(Outcome::Login)}),Err(e)=>self.error=e}
                }
                ui.add_space(8.0);ui.small("Original fixtures use generated test patterns and tones. No licensed media is included.");
                }); }); });
            }else{
                if !wide { self.navigation(ui, true); ui.add_space(12.0); }
                ui.add_space(12.0);
                ui.heading(Self::section_title(self.model.section));
                ui.label(match self.model.section { Section::Home=>"Picked from your account, progress and preferences",Section::Sports=>"Your sports and local broadcasts",Section::Movies=>"Films in your library",Section::Shows=>"Series and episodes",Section::Music=>"Albums, artists and tracks",Section::Guide=>"Your live channels and program guide",Section::Search=>"Search all media in your account",Section::Collections=>"Your libraries and custom groups",Section::Account=>"Your account and connected library",Section::Queue=>"Your music, up next",Section::Discovery=>"Search your server’s discovery catalog" });
                ui.add_space(12.0);
                if matches!(self.model.section,Section::Search|Section::Discovery) {
                    ui.horizontal(|ui| { let response=ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Title, artist or keyword").min_size(egui::vec2(0.0,44.0)).desired_width((ui.available_width()-90.0).max(40.0)));
                        if ui.button("Search").clicked() || (response.lost_focus() && ui.input(|i|i.key_pressed(egui::Key::Enter))) { if self.model.section==Section::Search {self.run_search(ctx,true);}else{self.browse(ctx,None);} }
                    });
                    if self.model.section==Section::Search { self.search_controls(ui); }
                }
                #[cfg(not(target_arch="wasm32"))]
                if self.model.section==Section::Account { ui.collapsing("Advanced playback settings",|ui|{ui.label("HTTPS by default. Optional direct Iroh requires a verified pinned endpoint and address. Failure does not silently switch routes.");ui.label("Pinned Iroh endpoint ID");ui.text_edit_singleline(&mut self.peer_id);ui.label("Verified numeric address");ui.text_edit_singleline(&mut self.peer_address);
if ui.button("Stop playback").clicked()&& let Some(s)=&self.playback{s.send(PlaybackCommand::Stop);}}); }
                #[cfg(target_arch="wasm32")]
                if self.model.section==Section::Account { ui.collapsing("Connection", |ui| { ui.small(if self.model.fixture {"Original fixture account"} else {"Your account uses its authenticated server connection."}); }); }
                ui.add_space(16.0);ui.vertical(|ui|{
                    if self.model.section==Section::Account {
                        ui.horizontal_wrapped(|ui| { if ui.button("Music queue").clicked() {self.open_section(ctx,Section::Queue);}if ui.button("Discover").clicked(){self.open_section(ctx,Section::Discovery);} });
                        ui.heading("Home personalization");ui.label("Viewing progress and favorites shape Home on your account's server.");
                        if let Some(generation)=self.home.history_generation { ui.label("Submitted searches and selected results also help. History stays with this account.");if ui.add_enabled(self.pending.is_none(),egui::Button::new("Clear search history")).clicked(){self.call(ctx,"historyClear","DELETE","Bluefin/Home/Searches".into(),vec![],Some(serde_json::json!({"generation":generation})));} }
                    }else if self.model.section==Section::Queue{
                        ui.label(format!("Revision {} · {} tracks",self.model.queue.revision,self.model.queue.queue.item_ids.len()));
                        for (index,id) in self.model.queue.queue.item_ids.clone().iter().enumerate(){ui.horizontal_wrapped(|ui|{ui.label(format!("{}  {}",index+1,id));
if ui.button("Remove").clicked(){self.model.queue.queue.item_ids.remove(index);self.model.queue.queue.cursor=None;self.model.queue_dirty=true;}
if ui.button("Play").clicked(){self.play_selected(Item{id:*id,name:"Queued track".into(),kind:"Audio".into(),overview:String::new(),start_date:None,end_date:None,channel_name:None,channel_id:None,..Default::default()},Selection::default(),Some(index));}});}
                        if ui.checkbox(&mut self.model.queue.queue.shuffle,"Shuffle").changed(){self.model.queue_dirty=true;}
                        egui::ComboBox::from_label("Repeat").selected_text(format!("{:?}",self.model.queue.queue.repeat)).show_ui(ui,|ui|{for mode in [bluefin_contract::RepeatMode::Off,bluefin_contract::RepeatMode::All,bluefin_contract::RepeatMode::One]{if ui.selectable_value(&mut self.model.queue.queue.repeat,mode,format!("{mode:?}")).changed(){self.model.queue_dirty=true;}}});
                        if ui.add_enabled(self.pending.is_none()&&self.model.capabilities.as_ref().is_some_and(|c|c.music_queue_sync),egui::Button::new("Save queue to account")).clicked(){let update=UpdateMusicQueue{protocol_version:PROTOCOL_VERSION,expected_revision:self.model.queue.revision,queue:self.model.queue.queue.clone()};self.call(ctx,"queue","PUT","Bluefin/Music/Queue".into(),vec![],Some(serde_json::to_value(update).unwrap()));}
                        if ui.button("Reload queue, discard local edits").clicked(){self.model.queue_dirty=false;self.browse(ctx,None);}
                    }else if self.model.section==Section::Discovery{
                        if let Some(page)=self.discovery.clone(){for item in page.results{ui.heading(item.title);ui.label(item.overview);ui.label(item.availability);for action in item.actions{if ui.button(action.label).clicked(){self.call(ctx,"form","GET",format!("Bluefin/Discovery/Actions/{}",action.id),vec![],None);}}ui.separator();}}
                        if let Some(form)=self.form.clone(){ui.heading(&form.title);ui.label(&form.notice);for choice in &form.choices{let mut checked=self.selection.contains(&choice.id);
if ui.checkbox(&mut checked,&choice.label).changed(){if checked{self.selection.push(choice.id.clone());}else{self.selection.retain(|s|s!=&choice.id);}}}ui.checkbox(&mut self.confirmed,"I confirm I am authorized to use this media");
if ui.add_enabled(self.pending.is_none()&&self.confirmed&&(!form.requires_selection||!self.selection.is_empty()),egui::Button::new("Submit server action")).clicked(){let body=bluefin_contract::discovery::SubmitAction{action_id:form.id,selections:self.selection.clone(),authorized_use_confirmed:self.confirmed};self.call(ctx,"receipt","POST","Bluefin/Discovery/Requests".into(),vec![],Some(serde_json::to_value(body).unwrap()));}}
                    }else{
                        if !self.previous_shelves.is_empty() && self.model.detail.is_none()
                            && ui.button("Back").clicked() { self.back_to_shelf(); }
                        if let Some(item)=self.model.detail.clone(){
                            egui::Frame::new().fill(ui.visuals().faint_bg_color).corner_radius(14).inner_margin(20).show(ui, |ui| {
                                ui.small("SELECTED ITEM");ui.heading(&item.name);ui.label(&item.overview);ui.add_space(8.0);
                                ui.horizontal_wrapped(|ui| {
                                    if item.playable(){if ui.button("Play").clicked(){self.play(item.clone());}
                                    if item.kind=="Audio"&&ui.button("Add to queue").clicked()&& let Err(e)=self.model.enqueue(&item){self.error=e;}}
                                    if item.folder()&&ui.button("Browse collection").clicked(){self.browse(ctx,Some(item.clone()));}
                                    if ui.button("Back").clicked(){self.back_to_shelf();}
                                });
                            });ui.add_space(16.0);
                        }
                        if self.model.section==Section::Home && self.previous_shelves.is_empty() && self.model.detail.is_none() && !self.home.shelves.is_empty() {
                            if let Some(item)=self.home.featured.clone() {self.featured(ui,&item);ui.add_space(20.0);}
                            for shelf in self.home.shelves.clone() {ui.heading(&shelf.title);ui.add_space(8.0);self.media_grid(ui,&shelf.items);ui.add_space(20.0);}
                        } else if self.model.section==Section::Search && self.previous_shelves.is_empty() && self.model.detail.is_none() {
                            for (index,shelf) in self.search_results.shelves.clone().into_iter().enumerate() {ui.heading(shelf.lane.title());self.media_grid(ui,&shelf.page.items);if !shelf.warning.is_empty(){ui.label(&shelf.warning);}if shelf.page.more() && ui.add_enabled(self.pending.is_none(),egui::Button::new("Load more")).clicked(){self.load_more(ctx,Some(index));}ui.add_space(16.0);}
                        } else {let rows=self.model.rows.clone();self.media_grid(ui,&rows);if self.page.more() && ui.add_enabled(self.pending.is_none(),egui::Button::new("Load more")).clicked(){self.load_more(ctx,None);} }
                        if self.model.rows.is_empty()&&self.pending.is_none(){ui.label(if self.model.section==Section::Search && self.query.is_empty(){"Enter a title, artist or keyword, or choose a refinement."}else{"No media to show for this account and view."});}
                    }
                });
            }
            for service in &self.retired { let status=service.snapshot(); ui.label(&status.message); }
            if !self.error.is_empty(){ui.colored_label(ui.visuals().error_fg_color,&self.error);}
            });
        });
    }
}
impl Drop for Bluefin {
    fn drop(&mut self) {
        self.cancel();
        if let Some(service) = &self.playback {
            service.send(PlaybackCommand::Logout);
        }
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use eframe::App;

    fn frame(
        app: &mut Bluefin,
        ctx: &egui::Context,
        width: f32,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        frame_at(app, ctx, width, 800.0, events)
    }
    fn frame_at(
        app: &mut Bluefin,
        ctx: &egui::Context,
        width: f32,
        height: f32,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, height),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
        )
    }
    fn texts(shape: &egui::epaint::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::epaint::Shape::Text(t) => out.push((
                t.galley.text().to_owned(),
                t.galley.rect.translate(t.pos.to_vec2()),
            )),
            egui::epaint::Shape::Vec(v) => {
                for s in v {
                    texts(s, out);
                }
            }
            _ => {}
        }
    }
    fn labels(output: &egui::FullOutput) -> Vec<(String, egui::Rect)> {
        let mut result = vec![];
        for s in &output.shapes {
            texts(&s.shape, &mut result);
        }
        result
    }
    #[test]
    fn short_sidebar_keyboard_focus_reveals_and_activates_account() {
        for (width, height) in [(1280.0, 320.0), (760.0, 320.0), (1280.0, 220.0)] {
            let ctx = egui::Context::default();
            let mut app = Bluefin::default();
            app.model.fixture = true;
            app.model.rows = app.model.fixture_rows();
            let _ = frame_at(&mut app, &ctx, width, height, vec![]);
            let _ = frame_at(&mut app, &ctx, width, height, vec![]);
            let key = |key, pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            let mut reached_account = false;
            for _ in 0..16 {
                let _ = frame_at(
                    &mut app,
                    &ctx,
                    width,
                    height,
                    vec![key(egui::Key::Tab, true)],
                );
                let _ = frame_at(
                    &mut app,
                    &ctx,
                    width,
                    height,
                    vec![key(egui::Key::Tab, false)],
                );
                for _ in 0..30 {
                    let _ = frame_at(&mut app, &ctx, width, height, vec![]);
                }
                let output = frame_at(&mut app, &ctx, width, height, vec![]);
                let response = ctx
                    .memory(|m| m.focused())
                    .and_then(|id| ctx.read_response(id))
                    .unwrap();
                let names: Vec<_> = labels(&output)
                    .into_iter()
                    .filter(|(_, rect)| response.rect.contains(rect.center()))
                    .map(|(label, _)| label)
                    .collect();
                if names.iter().any(|name| name == "Account") {
                    reached_account = true;
                    break;
                }
            }
            assert!(
                reached_account,
                "keyboard never revealed Account at {width}x{height}"
            );
            let focused = ctx.memory(|m| m.focused()).expect("keyboard focus lost");
            let response = ctx
                .read_response(focused)
                .expect("focused navigation absent");
            assert!(
                response
                    .interact_rect
                    .contains_rect(response.rect.shrink(1.0)),
                "focused navigation is clipped at {width}x{height}: {:?} vs {:?}",
                response.rect,
                response.interact_rect
            );
            let _ = frame_at(
                &mut app,
                &ctx,
                width,
                height,
                vec![key(egui::Key::Enter, true)],
            );
            let _ = frame_at(
                &mut app,
                &ctx,
                width,
                height,
                vec![key(egui::Key::Enter, false)],
            );
            assert!(
                app.model.section == Section::Account,
                "keyboard did not activate Account"
            );
        }
    }
    #[test]
    fn short_desktop_sidebar_scroll_exposes_and_activates_account() {
        for (width, height) in [
            (1280.0, 400.0),
            (760.0, 400.0),
            (1280.0, 220.0),
            (760.0, 220.0),
        ] {
            let ctx = egui::Context::default();
            let mut app = Bluefin::default();
            app.model.fixture = true;
            app.model.rows = app.model.fixture_rows();
            let _ = frame_at(&mut app, &ctx, width, height, vec![]);
            let _ = frame_at(
                &mut app,
                &ctx,
                width,
                height,
                vec![
                    egui::Event::PointerMoved(egui::pos2(100.0, height * 0.5)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -1000.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            let mut output = frame_at(&mut app, &ctx, width, height, vec![]);
            for _ in 0..20 {
                output = frame_at(&mut app, &ctx, width, height, vec![]);
            }
            let rect = labels(&output)
                .into_iter()
                .find(|(text, _)| text == "Account")
                .unwrap_or_else(|| panic!("Account is unreachable at {width}x{height}"))
                .1;
            assert!(
                rect.top() >= 0.0 && rect.bottom() <= height,
                "clipped navigation at {width}x{height}: {rect:?}"
            );
            let pos = rect.center();
            let click = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let _ = frame_at(
                &mut app,
                &ctx,
                width,
                height,
                vec![egui::Event::PointerMoved(pos), click(true)],
            );
            let _ = frame_at(&mut app, &ctx, width, height, vec![click(false)]);
            assert!(
                app.model.section == Section::Account,
                "Account did not activate at {width}x{height}"
            );
        }
    }
    #[test]
    fn narrow_and_desktop_layout_keep_navigation_and_labels_inside_viewport() {
        for width in [320.0, 390.0, 759.0, 760.0, 1280.0] {
            for connected in [false, true] {
                let ctx = egui::Context::default();
                let mut app = Bluefin::default();
                app.model.fixture = connected;
                app.model.rows = app.model.fixture_rows();
                for item in &mut app.model.rows {
                    item.name = "A long library title with multiple words / Une très longue édition de la bibliothèque".into();
                }
                let _ = frame(&mut app, &ctx, width, vec![]);
                let output = frame(&mut app, &ctx, width, vec![]);
                let labels = labels(&output);
                for (text, rect) in &labels {
                    assert!(
                        rect.left() >= -1.0 && rect.right() <= width + 1.0,
                        "{width}: {text:?} overflows {rect:?}"
                    );
                }
                for required in if connected {
                    vec![
                        "Home",
                        "Sports",
                        "TV Shows",
                        "Movies",
                        "Live TV",
                        "Music",
                        "Search",
                        "Collections",
                        "Account",
                        "Sign out",
                    ]
                } else {
                    vec![
                        "Connect to your server",
                        "Server address",
                        "Continue",
                        "Explore original fixtures",
                    ]
                } {
                    assert!(
                        labels.iter().any(|(text, _)| text == required),
                        "missing {required} at {width}"
                    );
                }
                let _ = frame(
                    &mut app,
                    &ctx,
                    width,
                    vec![egui::Event::Key {
                        key: egui::Key::Tab,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
                assert!(
                    ctx.memory(|m| m.focused().is_some()),
                    "no keyboard focus at {width}"
                );
            }
        }
    }
    #[test]
    fn media_tile_click_selects_real_item_and_exposes_play_action() {
        let ctx = egui::Context::default();
        let mut app = Bluefin::default();
        app.model.fixture = true;
        app.model.rows = app.model.fixture_rows();
        let _ = frame(&mut app, &ctx, 1280.0, vec![]);
        let output = frame(&mut app, &ctx, 1280.0, vec![]);
        let pos = labels(&output)
            .into_iter()
            .find(|(text, _)| text == "Paper moon harbor")
            .unwrap()
            .1
            .center();
        let pointer = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = frame(
            &mut app,
            &ctx,
            1280.0,
            vec![egui::Event::PointerMoved(pos), pointer(true)],
        );
        let _ = frame(&mut app, &ctx, 1280.0, vec![pointer(false)]);
        assert_eq!(app.model.detail.as_ref().unwrap().id, app.model.rows[0].id);
        let output = frame(&mut app, &ctx, 1280.0, vec![]);
        assert!(labels(&output).iter().any(|(text, _)| text == "Play"));
    }
}
