// ============================================================================
// xhml - A custom Minecraft launcher (single-file edition)
// Dark blue egui theme, instance manager, Modrinth search/download,
// local dev profile, and a structural Java process runner.
// ============================================================================

use eframe::egui::{self, Color32, Visuals};
use serde::{Deserialize, Serialize};
use std::fs;
use std::fs::File;
use std::io::copy;
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

// ============================================================================
// THEME
// ============================================================================

fn apply_theme(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();

    // Deep navy backgrounds
    visuals.panel_fill = Color32::from_rgb(10, 18, 35);
    visuals.window_fill = Color32::from_rgb(14, 24, 48);

    // Dark blue accents
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(20, 35, 65);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(25, 45, 80);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(35, 60, 100);
    visuals.widgets.active.bg_fill = Color32::from_rgb(45, 80, 130);

    // Clean white / light-gray typography
    visuals.widgets.noninteractive.fg_stroke.color = Color32::from_rgb(220, 225, 235);
    visuals.widgets.inactive.fg_stroke.color = Color32::from_rgb(200, 210, 225);
    visuals.widgets.hovered.fg_stroke.color = Color32::WHITE;
    visuals.widgets.active.fg_stroke.color = Color32::WHITE;

    // Selection highlight
    visuals.selection.bg_fill = Color32::from_rgb(40, 90, 160);

    ctx.set_visuals(visuals);
}

// ============================================================================
// AUTH
// ============================================================================

trait AuthProvider {
    fn username(&self) -> &str;
    fn uuid(&self) -> &str;
    fn access_token(&self) -> &str;
    fn user_type(&self) -> &str;
    fn is_demo(&self) -> bool;
}

struct LocalDevProfile {
    username: String,
    uuid: String,
}

impl LocalDevProfile {
    fn new(username: &str) -> Self {
        let namespace = Uuid::new_v3(&Uuid::NAMESPACE_DNS, b"xhml.local.dev.namespace");
        let uuid = Uuid::new_v3(&namespace, format!("OfflinePlayer:{}", username).as_bytes());
        Self {
            username: username.to_string(),
            uuid: uuid.to_string(),
        }
    }
}

impl AuthProvider for LocalDevProfile {
    fn username(&self) -> &str {
        &self.username
    }
    fn uuid(&self) -> &str {
        &self.uuid
    }
    fn access_token(&self) -> &str {
        "0"
    }
    fn user_type(&self) -> &str {
        "legacy"
    }
    fn is_demo(&self) -> bool {
        true
    }
}

// ============================================================================
// INSTANCE MANAGER
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Instance {
    name: String,
    mc_version: String,
    fabric_loader: String,
    game_dir: PathBuf,
}

impl Instance {
    fn new(name: &str, mc_version: &str, fabric_loader: &str, base: &Path) -> Self {
        let game_dir = base.join("instances").join(name);
        Self {
            name: name.to_string(),
            mc_version: mc_version.to_string(),
            fabric_loader: fabric_loader.to_string(),
            game_dir,
        }
    }

    fn ensure_dirs(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.game_dir)?;
        fs::create_dir_all(self.game_dir.join("mods"))?;
        Ok(())
    }

    fn save(&self) -> std::io::Result<()> {
        self.ensure_dirs()?;
        let meta_path = self.game_dir.join("instance.json");
        let json = serde_json::to_string_pretty(self)?;
        fs::write(meta_path, json)
    }

    fn load_all(base: &Path) -> Vec<Instance> {
        let instances_dir = base.join("instances");
        let mut result = Vec::new();
        if let Ok(entries) = fs::read_dir(&instances_dir) {
            for entry in entries.flatten() {
                let meta = entry.path().join("instance.json");
                if let Ok(content) = fs::read_to_string(meta) {
                    if let Ok(inst) = serde_json::from_str::<Instance>(&content) {
                        result.push(inst);
                    }
                }
            }
        }
        result
    }
}

// ============================================================================
// MODRINTH CLIENT
// ============================================================================

#[derive(Debug, Deserialize)]
struct SearchResponse {
    hits: Vec<ModHit>,
}

#[derive(Debug, Deserialize)]
struct ModHit {
    #[allow(dead_code)]
    project_id: String,
    slug: String,
    title: String,
    description: String,
}

#[derive(Debug, Deserialize)]
struct ProjectVersion {
    files: Vec<ModFile>,
}

#[derive(Debug, Deserialize)]
struct ModFile {
    url: String,
    filename: String,
}

struct ModrinthClient {
    client: reqwest::blocking::Client,
}

impl ModrinthClient {
    fn new() -> Self {
        Self {
            client: reqwest::blocking::Client::builder()
                .user_agent("xhml/0.1 (local dev)")
                .build()
                .expect("Failed to build HTTP client"),
        }
    }

    fn search(&self, query: &str, limit: usize) -> Result<SearchResponse, String> {
        let url = "https://api.modrinth.com/v2/search";
        let resp = self
            .client
            .get(url)
            .query(&[
                ("query", query),
                ("limit", &limit.to_string()),
                ("facets", "[["project_type:mod"]]"),
            ])
            .send()
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;

        resp.json::<SearchResponse>().map_err(|e| e.to_string())
    }

    fn download_mod(&self, slug: &str, mods_dir: &Path) -> Result<String, String> {
        let version_url = format!("https://api.modrinth.com/v2/project/{}/version", slug);
        let versions = self
            .client
            .get(&version_url)
            .send()
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json::<Vec<ProjectVersion>>()
            .map_err(|e| e.to_string())?;

        let version = versions.first().ok_or("No versions found")?;
        let file = version.files.first().ok_or("No file found")?;

        let mut resp = self
            .client
            .get(&file.url)
            .send()
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;

        fs::create_dir_all(mods_dir).map_err(|e| e.to_string())?;
        let target = mods_dir.join(&file.filename);
        let mut out = File::create(&target).map_err(|e| e.to_string())?;
        copy(&mut resp, &mut out).map_err(|e| e.to_string())?;

        Ok(file.filename.clone())
    }
}

// ============================================================================
// PROCESS RUNNER
// ============================================================================

struct LaunchConfig<'a> {
    java_path: &'a str,
    instance: &'a Instance,
    auth: &'a dyn AuthProvider,
    jvm_args: Vec<String>,
}

fn build_command(cfg: &LaunchConfig) -> Result<Command, String> {
    let instance = cfg.instance;
    let game_dir = &instance.game_dir;

    let main_class = "net.fabricmc.loader.impl.launch.knot.KnotClient";

    let classpath = format!(
        "{}/versions/{}/client.jar:{}/mods/*",
        game_dir.display(),
        instance.mc_version,
        game_dir.display()
    );

    let mut cmd = Command::new(cfg.java_path);

    for arg in &cfg.jvm_args {
        cmd.arg(arg);
    }

    cmd.arg("-cp").arg(&classpath).arg(main_class);

    cmd.arg("--username").arg(cfg.auth.username());
    cmd.arg("--uuid").arg(cfg.auth.uuid());
    cmd.arg("--accessToken").arg(cfg.auth.access_token());
    cmd.arg("--userType").arg(cfg.auth.user_type());
    cmd.arg("--version").arg(&instance.mc_version);
    cmd.arg("--gameDir").arg(game_dir);
    cmd.arg("--assetsDir").arg(game_dir.join("assets"));

    if cfg.auth.is_demo() {
        cmd.arg("--demo");
    }

    cmd.current_dir(game_dir);
    Ok(cmd)
}

fn launch(cfg: &LaunchConfig) -> Result<std::process::Child, String> {
    let mut cmd = build_command(cfg)?;
    cmd.spawn()
        .map_err(|e| format!("Failed to spawn Java: {}", e))
}

// ============================================================================
// APP / UI
// ============================================================================

struct XhmlApp {
    base_dir: PathBuf,
    instances: Vec<Instance>,
    selected_instance: Option<usize>,
    dev_username: String,
    dev_profile: Option<LocalDevProfile>,
    modrinth: ModrinthClient,
    search_query: String,
    search_results: Vec<ModHit>,
    status: String,
}

impl XhmlApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_theme(&cc.egui_ctx);

        let base_dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("xhml_data");
        fs::create_dir_all(&base_dir).ok();

        let instances = Instance::load_all(&base_dir);

        Self {
            base_dir,
            instances,
            selected_instance: None,
            dev_username: "DevPlayer".to_string(),
            dev_profile: None,
            modrinth: ModrinthClient::new(),
            search_query: String::new(),
            search_results: Vec::new(),
            status: "Ready.".to_string(),
        }
    }

    fn create_instance(&mut self) {
        let name = format!("fabric-{}", self.instances.len() + 1);
        let inst = Instance::new(&name, "1.21.1", "0.16.10", &self.base_dir);
        if inst.save().is_ok() {
            self.instances.push(inst);
            self.status = format!("Created instance '{}'", name);
        } else {
            self.status = "Failed to create instance directory.".into();
        }
    }

    fn apply_dev_profile(&mut self) {
        if self.dev_username.trim().is_empty() {
            self.status = "Username cannot be empty.".into();
            return;
        }
        let profile = LocalDevProfile::new(self.dev_username.trim());
        self.status = format!("Dev UUID: {}", profile.uuid);
        self.dev_profile = Some(profile);
    }

    fn run_search(&mut self) {
        match self.modrinth.search(&self.search_query, 10) {
            Ok(resp) => {
                self.search_results = resp.hits;
                self.status = format!("Found {} mods.", self.search_results.len());
            }
            Err(e) => self.status = format!("Search error: {}", e),
        }
    }

    fn download_selected_mod(&mut self, slug: &str) {
        let Some(idx) = self.selected_instance else {
            self.status = "Select an instance first.".into();
            return;
        };
        let mods_dir = self.instances[idx].game_dir.join("mods");
        match self.modrinth.download_mod(slug, &mods_dir) {
            Ok(filename) => self.status = format!("Downloaded: {}", filename),
            Err(e) => self.status = format!("Download error: {}", e),
        }
    }

    fn launch_selected(&mut self) {
        let Some(idx) = self.selected_instance else {
            self.status = "Select an instance first.".into();
            return;
        };
        let Some(profile) = &self.dev_profile else {
            self.status = "Apply a local dev profile first.".into();
            return;
        };

        let inst = &self.instances[idx];
        let cfg = LaunchConfig {
            java_path: "java",
            instance: inst,
            auth: profile,
            jvm_args: vec!["-Xmx2G".into()],
        };

        match launch(&cfg) {
            Ok(child) => self.status = format!("Launched! PID = {}", child.id()),
            Err(e) => self.status = format!("Launch failed: {}", e),
        }
    }
}

impl eframe::App for XhmlApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(egui::RichText::new("xhml").color(Color32::from_rgb(120, 180, 255)));
                ui.label("Minecraft Launcher");
            });
        });

        egui::SidePanel::left("instances").show(ctx, |ui| {
            ui.heading("Instances");
            if ui.button("+ New Fabric 1.21+ Instance").clicked() {
                self.create_instance();
            }
            ui.separator();

            for (i, inst) in self.instances.iter().enumerate() {
                let selected = self.selected_instance == Some(i);
                if ui.selectable_label(selected, &inst.name).clicked() {
                    self.selected_instance = Some(i);
                }
            }

            ui.separator();
            ui.heading("Local Dev Profile");
            ui.text_edit_singleline(&mut self.dev_username);
            if ui.button("Apply Dev Profile").clicked() {
                self.apply_dev_profile();
            }
            if let Some(p) = &self.dev_profile {
                ui.label(format!("UUID: {}", &p.uuid[..8]));
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Modrinth");
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.search_query);
                if ui.button("Search").clicked() {
                    self.run_search();
                }
            });

            egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                for hit in &self.search_results {
                    ui.horizontal(|ui| {
                        ui.label(&hit.title);
                        if ui.button("Download").clicked() {
                            let slug = hit.slug.clone();
                            self.download_selected_mod(&slug);
                        }
                    });
                    ui.label(egui::RichText::new(&hit.description).small());
                }
            });

            ui.separator();
            if ui.button("Launch Selected Instance").clicked() {
                self.launch_selected();
            }

            ui.separator();
            ui.label(&self.status);
        });
    }
}

// ============================================================================
// ENTRY POINT
// ============================================================================

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_title("xhml"),
        ..Default::default()
    };

    eframe::run_native(
        "xhml",
        options,
        Box::new(|cc| Ok(Box::new(XhmlApp::new(cc)))),
    )
}
