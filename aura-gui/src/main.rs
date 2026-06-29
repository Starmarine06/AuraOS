#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(dead_code)]

use eframe::egui;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Message {
    sender: String, // "User" or "Aura"
    content: String,
    #[serde(skip)]
    added_at: Option<Instant>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DaemonStatus {
    is_recording: bool,
    current_macro_name: String,
    model: String,
    available_macros: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    response: String,
}

struct AppInfo {
    name: &'static str,
    executable: &'static str,
    icon: &'static str,
}

const APPS: &[AppInfo] = &[
    AppInfo { name: "Firefox Web Browser", executable: "firefox", icon: "🌐" },
    AppInfo { name: "Alacritty Terminal", executable: "alacritty", icon: "💻" },
    AppInfo { name: "XFCE File Manager", executable: "thunar", icon: "📁" },
    AppInfo { name: "Calculator", executable: "galculator", icon: "🔢" },
    AppInfo { name: "Application Finder", executable: "xfce4-appfinder", icon: "🔍" },
];

fn eval_math(expr: &str) -> Option<f64> {
    let cleaned: String = expr.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.is_empty() || !cleaned.chars().all(|c| c.is_digit(10) || c == '+' || c == '-' || c == '*' || c == '/' || c == '.') {
        return None;
    }
    eval_math_recursive(&cleaned)
}

fn eval_math_recursive(cleaned: &str) -> Option<f64> {
    for op in &['+', '-', '*', '/'] {
        if let Some(pos) = cleaned.rfind(*op) {
            let left_str = &cleaned[..pos];
            let right_str = &cleaned[pos+1..];
            let left = if left_str.is_empty() { 0.0 } else { eval_math_recursive(left_str)? };
            let right = eval_math_recursive(right_str)?;
            return match op {
                '+' => Some(left + right),
                '-' => Some(left - right),
                '*' => Some(left * right),
                '/' => {
                    if right == 0.0 { None } else { Some(left / right) }
                }
                _ => None
            };
        }
    }
    cleaned.parse::<f64>().ok()
}

// Visual Helpers
fn multiply_alpha(color: egui::Color32, factor: f32) -> egui::Color32 {
    egui::Color32::from_rgba_premultiplied(
        (color.r() as f32 * factor) as u8,
        (color.g() as f32 * factor) as u8,
        (color.b() as f32 * factor) as u8,
        (color.a() as f32 * factor) as u8,
    )
}

fn lerp_color(c1: egui::Color32, c2: egui::Color32, t: f32) -> egui::Color32 {
    egui::Color32::from_rgba_premultiplied(
        (c1.r() as f32 + (c2.r() as f32 - c1.r() as f32) * t) as u8,
        (c1.g() as f32 + (c2.g() as f32 - c1.g() as f32) * t) as u8,
        (c1.b() as f32 + (c2.b() as f32 - c1.b() as f32) * t) as u8,
        (c1.a() as f32 + (c2.a() as f32 - c1.a() as f32) * t) as u8,
    )
}

fn paint_rounded_gradient(
    painter: &egui::Painter,
    rect: egui::Rect,
    top_color: egui::Color32,
    bottom_color: egui::Color32,
    corner_radius: f32,
) {
    let steps = 10;
    let step_h = rect.height() / steps as f32;
    for i in 0..steps {
        let t = i as f32 / (steps - 1) as f32;
        let color = lerp_color(top_color, bottom_color, t);
        let top = rect.top() + i as f32 * step_h;
        let bottom = if i == steps - 1 { rect.bottom() } else { top + step_h };
        let strip_rect = egui::Rect::from_x_y_ranges(rect.left()..=rect.right(), top..=bottom);
        
        let rounding = if i == 0 {
            egui::Rounding {
                nw: corner_radius,
                ne: corner_radius,
                sw: 0.0,
                se: 0.0,
            }
        } else if i == steps - 1 {
            egui::Rounding {
                nw: 0.0,
                ne: 0.0,
                sw: corner_radius,
                se: corner_radius,
            }
        } else {
            egui::Rounding::ZERO
        };
        
        painter.rect_filled(strip_rect, rounding, color);
    }
}

fn animated_button(
    ui: &mut egui::Ui,
    text: &str,
    normal_bg: egui::Color32,
    hover_bg: egui::Color32,
    text_color: egui::Color32,
    corner_radius: f32,
    opacity: f32,
) -> egui::Response {
    let button_padding = egui::vec2(12.0, 6.0);
    let text_style = egui::TextStyle::Button;
    let wrap_width = ui.available_width();
    let text_job = egui::WidgetText::from(text).into_galley(ui, None, wrap_width, text_style);
    let desired_size = text_job.size() + button_padding * 2.0;
    
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
    
    if ui.is_rect_visible(rect) {
        let hover_t = ui.ctx().animate_bool(response.id, response.hovered());
        let bg_color = lerp_color(normal_bg, hover_bg, hover_t);
        let final_bg = multiply_alpha(bg_color, opacity);
        let final_text_color = multiply_alpha(text_color, opacity);
        
        let painter = ui.painter();
        painter.rect_filled(rect, corner_radius, final_bg);
        
        let text_pos = rect.min + button_padding;
        painter.galley(text_pos, text_job, final_text_color);
    }
    
    response
}

fn paint_bouncing_dots(ui: &mut egui::Ui, center_y_offset: f32, opacity: f32) {
    let time = ui.input(|i| i.time);
    let dot_radius = 5.0;
    let dot_spacing = 15.0;
    let num_dots = 3;
    
    let size = egui::vec2(dot_spacing * (num_dots - 1) as f32 + dot_radius * 4.0, 30.0);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    
    let painter = ui.painter();
    let center = rect.center();
    
    for i in 0..num_dots {
        let delay = i as f64 * 0.35;
        let bounce = (time * 6.0 - delay).sin() as f32;
        let y_offset = if bounce > 0.0 { bounce * -6.0 } else { 0.0 };
        
        let dot_center = egui::pos2(
            center.x - (dot_spacing * (num_dots - 1) as f32 * 0.5) + i as f32 * dot_spacing,
            center.y + y_offset + center_y_offset,
        );
        
        let color = if i == 0 {
            egui::Color32::from_rgb(203, 166, 247) // Mauve
        } else if i == 1 {
            egui::Color32::from_rgb(180, 190, 254) // Lavender
        } else {
            egui::Color32::from_rgb(137, 180, 250) // Blue
        };
        
        let final_color = multiply_alpha(color, opacity);
        painter.circle_filled(dot_center, dot_radius, final_color);
        
        let glow_color = egui::Color32::from_rgba_premultiplied(color.r(), color.g(), color.b(), 40);
        let final_glow = multiply_alpha(glow_color, opacity);
        painter.circle_filled(dot_center, dot_radius + 3.0, final_glow);
    }
    
    ui.ctx().request_repaint();
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct BluetoothDevice {
    mac: String,
    name: String,
}

struct AuraApp {
    prompt: String,
    messages: Vec<Message>,
    is_waiting: bool,
    is_recording: bool,
    current_macro_name: String,
    available_macros: Vec<String>,
    model_name: String,
    status_visible: bool,
    visible: Arc<AtomicBool>,
    status_tx: tokio::sync::mpsc::UnboundedSender<String>,
    status_rx: tokio::sync::mpsc::UnboundedReceiver<String>,
    model_ready: bool,
    was_visible: bool,
    window_open_time: Option<Instant>,
    phone_mac: String,
    bluetooth_enabled: bool,
    is_connected: bool,
    is_scanning: bool,
    scanned_devices: Vec<BluetoothDevice>,
}

impl AuraApp {
    fn new(cc: &eframe::CreationContext<'_>, visible: Arc<AtomicBool>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.window_rounding = 16.0.into();
        visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(30, 30, 46);
        visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(49, 50, 68);
        visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(69, 71, 90);
        visuals.widgets.active.bg_fill = egui::Color32::from_rgb(88, 91, 112);
        visuals.widgets.noninteractive.fg_stroke.color = egui::Color32::from_rgb(205, 214, 244);
        cc.egui_ctx.set_visuals(visuals);

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        let messages = vec![Message {
            sender: "Aura".to_string(),
            content: "Hello! I am Aura, your system AI. How can I help you today? I have full root control and macro playback capabilities.".to_string(),
            added_at: Some(Instant::now()),
        }];

        // Periodically poll daemon status and bluetooth status
        let ctx_clone = cc.egui_ctx.clone();
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            loop {
                // 1. Daemon Status
                if let Ok(res) = reqwest::get("http://localhost:5050/api/status").await {
                    if let Ok(status) = res.json::<DaemonStatus>().await {
                        let status_json = serde_json::to_string(&status).unwrap_or_default();
                        let _ = tx_clone.send(status_json);
                        ctx_clone.request_repaint();
                    } else {
                        let _ = tx_clone.send("Error: unreachable".to_string());
                    }
                } else {
                    let _ = tx_clone.send("Error: unreachable".to_string());
                }

                // 2. Bluetooth Status
                if let Ok(res) = reqwest::get("http://localhost:5050/api/bluetooth/status").await {
                    #[derive(Deserialize)]
                    struct BtStatus { mac_address: Option<String>, enabled: bool, is_connected: bool }
                    if let Ok(bt) = res.json::<BtStatus>().await {
                        let bt_json = serde_json::json!({
                            "bluetooth_status": {
                                "mac": bt.mac_address.unwrap_or_default(),
                                "enabled": bt.enabled,
                                "is_connected": bt.is_connected
                            }
                        }).to_string();
                        let _ = tx_clone.send(bt_json);
                        ctx_clone.request_repaint();
                    }
                }

                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        });

        Self {
            prompt: String::new(),
            messages,
            is_waiting: false,
            is_recording: false,
            current_macro_name: String::new(),
            available_macros: Vec::new(),
            model_name: "Detecting...".to_string(),
            status_visible: false,
            visible,
            status_tx: tx,
            status_rx: rx,
            model_ready: false,
            was_visible: false,
            window_open_time: None,
            phone_mac: String::new(),
            bluetooth_enabled: false,
            is_connected: false,
            is_scanning: false,
            scanned_devices: Vec::new(),
        }
    }

    fn send_chat(&mut self, ctx: &egui::Context) {
        if self.prompt.trim().is_empty() || self.is_waiting {
            return;
        }

        let query = self.prompt.trim().to_string();

        // 1. Intercept math calculations
        if let Some(res) = eval_math(&query) {
            self.messages.push(Message {
                sender: "User".to_string(),
                content: query.clone(),
                added_at: Some(Instant::now()),
            });
            self.messages.push(Message {
                sender: "Aura".to_string(),
                content: format!("= {}", res),
                added_at: Some(Instant::now()),
            });
            self.prompt.clear();
            return;
        }

        // 2. Intercept local application launches
        let mut matched_app = None;
        for app in APPS {
            if app.executable.eq_ignore_ascii_case(&query) || app.name.to_lowercase().starts_with(&query.to_lowercase()) {
                matched_app = Some(app);
                break;
            }
        }
        if let Some(app) = matched_app {
            let _ = std::process::Command::new(app.executable).spawn();
            self.messages.push(Message {
                sender: "User".to_string(),
                content: format!("Launch {}", app.name),
                added_at: Some(Instant::now()),
            });
            self.messages.push(Message {
                sender: "Aura".to_string(),
                content: format!("Launched {} ({})", app.name, app.executable),
                added_at: Some(Instant::now()),
            });
            self.prompt.clear();
            return;
        }

        let user_prompt = self.prompt.clone();
        self.messages.push(Message {
            sender: "User".to_string(),
            content: user_prompt.clone(),
            added_at: Some(Instant::now()),
        });
        self.prompt.clear();
        self.is_waiting = true;

        let ctx_clone = ctx.clone();
        let tx_clone = self.status_tx.clone();

        tokio::spawn(async move {
            #[derive(Serialize)]
            struct ChatReq { prompt: String }
            let client = reqwest::Client::new();
            
            let result = client.post("http://localhost:5050/api/chat")
                .json(&ChatReq { prompt: user_prompt })
                .send()
                .await;

            match result {
                Ok(res) => {
                    if let Ok(chat_res) = res.json::<ChatResponse>().await {
                        let response_msg = Message {
                            sender: "Aura".to_string(),
                            content: chat_res.response,
                            added_at: Some(Instant::now()),
                        };
                        let response_json = serde_json::json!({ "chat_response": response_msg }).to_string();
                        let _ = tx_clone.send(response_json);
                    } else {
                        let _ = tx_clone.send("Error: Failed to parse AI response".to_string());
                    }
                }
                Err(e) => {
                    let _ = tx_clone.send(format!("Error: Daemon unreachable ({})", e));
                }
            }
            ctx_clone.request_repaint();
        });
    }

    fn save_bluetooth_settings(&self) {
        let mac = if self.phone_mac.is_empty() { None } else { Some(self.phone_mac.clone()) };
        let enabled = self.bluetooth_enabled;
        
        tokio::spawn(async move {
            #[derive(Serialize)]
            struct BtSetup { mac_address: Option<String>, enabled: bool }
            let client = reqwest::Client::new();
            let _ = client.post("http://localhost:5050/api/bluetooth/setup")
                .json(&BtSetup { mac_address: mac, enabled })
                .send()
                .await;
        });
    }
}

impl eframe::App for AuraApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Read daemon updates and responses
        while let Ok(msg) = self.status_rx.try_recv() {
            if let Ok(status) = serde_json::from_str::<DaemonStatus>(&msg) {
                self.is_recording = status.is_recording;
                self.current_macro_name = status.current_macro_name;
                self.model_name = status.model.clone();
                self.available_macros = status.available_macros;

                if !self.model_name.is_empty() && self.model_name != "Detecting..." && self.model_name != "No model detected" {
                    self.model_ready = true;
                }
            } else if let Ok(val) = serde_json::from_str::<serde_json::Value>(&msg) {
                if let Some(chat_res) = val.get("chat_response") {
                    if let Ok(msg) = serde_json::from_value::<Message>(chat_res.clone()) {
                        let mut msg_with_time = msg;
                        msg_with_time.added_at = Some(Instant::now());
                        self.messages.push(msg_with_time);
                    }
                    self.is_waiting = false;
                } else if let Some(bt_status) = val.get("bluetooth_status") {
                    self.phone_mac = bt_status.get("mac").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                    self.bluetooth_enabled = bt_status.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
                    self.is_connected = bt_status.get("is_connected").and_then(|v| v.as_bool()).unwrap_or(false);
                } else if let Some(bt_devices) = val.get("bluetooth_devices") {
                    if let Ok(devs) = serde_json::from_value::<Vec<BluetoothDevice>>(bt_devices.clone()) {
                        self.scanned_devices = devs;
                    }
                }
            } else if msg.starts_with("Error:") {
                if msg.contains("unreachable") || msg.contains("Connection refused") {
                    self.model_name = "Detecting...".to_string();
                    self.model_ready = false;
                } else {
                    self.messages.push(Message {
                        sender: "Aura".to_string(),
                        content: msg,
                        added_at: Some(Instant::now()),
                    });
                }
                self.is_waiting = false;
            } else if msg == "ScanFinished" {
                self.is_scanning = false;
            }
        }

        // Sync window visibility
        let is_visible = self.visible.load(Ordering::Relaxed);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(is_visible));
        if !is_visible {
            self.was_visible = false;
            self.window_open_time = None;
            return;
        }

        if !self.was_visible {
            self.was_visible = true;
            self.window_open_time = Some(Instant::now());
        }

        let open_progress = if let Some(open_time) = self.window_open_time {
            let elapsed = open_time.elapsed().as_secs_f32();
            let duration = 0.3; // 300 ms
            if elapsed < duration {
                ctx.request_repaint();
                let t = elapsed / duration;
                1.0 - (1.0 - t).powi(3) // Cubic ease out
            } else {
                1.0
            }
        } else {
            1.0
        };

        // Draw transparent CentralPanel
        let panel_frame = egui::Frame::none()
            .fill(egui::Color32::TRANSPARENT)
            .inner_margin(12.0);

        egui::CentralPanel::default().frame(panel_frame).show(ctx, |ui| {
            let window_y_offset = (1.0 - open_progress) * 35.0;
            let rect = ui.max_rect();
            let painter = ui.painter();
            
            // Draw background gradient (translucent glassmorphism)
            let top_bg = multiply_alpha(egui::Color32::from_rgba_premultiplied(20, 20, 32, 170), open_progress);
            let bottom_bg = multiply_alpha(egui::Color32::from_rgba_premultiplied(13, 13, 23, 170), open_progress);
            paint_rounded_gradient(painter, rect, top_bg, bottom_bg, 16.0);
            
            // Outer glowing border
            let border_color = multiply_alpha(egui::Color32::from_rgb(203, 166, 247), open_progress * 0.15);
            painter.rect_stroke(rect, 16.0, egui::Stroke::new(1.5, border_color));

            ui.add_space(window_y_offset);

            if !self.model_ready {
                // Glassmorphic Loading Screen
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.heading(egui::RichText::new("AuraOS AI").size(32.0).strong().color(multiply_alpha(egui::Color32::from_rgb(203, 166, 247), open_progress)));
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("Initializing Local AI Daemon...").size(16.0).color(multiply_alpha(egui::Color32::from_rgb(205, 214, 244), open_progress)));
                    ui.add_space(35.0);

                    paint_bouncing_dots(ui, 0.0, open_progress);
                    ui.add_space(35.0);

                    let status_text = if self.model_name == "Detecting..." {
                        "Waiting for background system daemon to boot..."
                    } else if self.model_name.is_empty() || self.model_name == "No model detected" {
                        "Booting local Ollama server & allocating memory..."
                    } else {
                        &format!("Spinning up model: {} (loading weights to CPU)...", self.model_name)
                    };

                    ui.label(egui::RichText::new(status_text).size(14.0).color(multiply_alpha(egui::Color32::from_rgb(166, 173, 200), open_progress)));
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("The system interface will unlock as soon as the model is ready.").size(11.0).italics().color(multiply_alpha(egui::Color32::from_rgb(108, 112, 134), open_progress)));

                    ui.add_space(40.0);
                    if animated_button(ui, "❌ Close", egui::Color32::from_rgb(49, 50, 68), egui::Color32::from_rgb(243, 139, 168), egui::Color32::from_rgb(205, 214, 244), 8.0, open_progress).clicked() {
                        self.visible.store(false, Ordering::Relaxed);
                    }
                    ui.add_space(50.0);
                });
            } else {
                // Glassmorphic Header (macOS & Windows Hybrid)
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);
                    
                    // macOS traffic light buttons on top-left
                    // Red circular close button
                    let (red_rect, red_res) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::click());
                    let red_color = if red_res.hovered() { egui::Color32::from_rgb(255, 95, 86) } else { egui::Color32::from_rgb(237, 106, 94) };
                    ui.painter().circle_filled(red_rect.center(), 6.0, multiply_alpha(red_color, open_progress));
                    if red_res.clicked() {
                        self.visible.store(false, Ordering::Relaxed);
                    }

                    // Yellow circular settings button
                    let (yellow_rect, yellow_res) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::click());
                    let yellow_color = if yellow_res.hovered() { egui::Color32::from_rgb(255, 189, 46) } else { egui::Color32::from_rgb(245, 181, 79) };
                    ui.painter().circle_filled(yellow_rect.center(), 6.0, multiply_alpha(yellow_color, open_progress));
                    if yellow_res.clicked() {
                        self.status_visible = !self.status_visible;
                    }

                    // Green circular clear chat button
                    let (green_rect, green_res) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::click());
                    let green_color = if green_res.hovered() { egui::Color32::from_rgb(39, 201, 63) } else { egui::Color32::from_rgb(98, 196, 84) };
                    ui.painter().circle_filled(green_rect.center(), 6.0, multiply_alpha(green_color, open_progress));
                    if green_res.clicked() {
                        self.messages.truncate(1);
                    }
                    
                    ui.add_space(8.0);
                    ui.heading(egui::RichText::new("✦ Aura OS AI").color(multiply_alpha(egui::Color32::from_rgb(203, 166, 247), open_progress)).strong());
                    
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Windows-style metadata on the right
                        let time = ui.input(|i| i.time);
                        let pulse = (time * 4.0).sin() as f32 * 0.4 + 0.6;
                        let dot_color = egui::Color32::from_rgba_premultiplied(166, 227, 161, (pulse * 255.0) as u8);
                        let final_dot_color = multiply_alpha(dot_color, open_progress);
                        let final_glow_color = multiply_alpha(egui::Color32::from_rgb(166, 227, 161), open_progress * 0.15);
                        
                        ui.horizontal(|ui| {
                            let (dot_rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                            ui.painter().circle_filled(dot_rect.center(), 4.0, final_dot_color);
                            ui.painter().circle_filled(dot_rect.center(), 6.0, final_glow_color);
                            
                            ui.add_space(2.0);
                            ui.label(egui::RichText::new(format!("Model: {}", self.model_name)).color(multiply_alpha(egui::Color32::from_rgb(166, 173, 200), open_progress)).size(11.0));
                        });
                    });
                });

                ui.separator();

                ui.horizontal(|ui| {
                    if self.status_visible {
                        // Left Settings Panel (200px width)
                        ui.allocate_ui_with_layout(
                            egui::vec2(200.0, ui.available_height()),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.label(egui::RichText::new("Bluetooth Proximity").strong().color(egui::Color32::from_rgb(205, 214, 244)));
                                ui.add_space(4.0);
                                
                                let mut enabled = self.bluetooth_enabled;
                                if ui.checkbox(&mut enabled, "Proximity Lock").changed() {
                                    self.bluetooth_enabled = enabled;
                                    self.save_bluetooth_settings();
                                }
                                
                                ui.add_space(6.0);
                                ui.label(egui::RichText::new(format!("MAC: {}", if self.phone_mac.is_empty() { "None" } else { &self.phone_mac })).size(11.0).color(egui::Color32::from_rgb(166, 173, 200)));
                                ui.label(egui::RichText::new(format!("State: {}", if self.is_connected { "Connected" } else { "Disconnected" })).size(11.0).color(if self.is_connected { egui::Color32::from_rgb(166, 227, 161) } else { egui::Color32::from_rgb(243, 139, 168) }));
                                
                                ui.add_space(8.0);
                                
                                if self.is_scanning {
                                    ui.horizontal(|ui| {
                                        paint_bouncing_dots(ui, 0.0, open_progress);
                                        ui.label(egui::RichText::new("Scanning...").size(11.0));
                                    });
                                } else {
                                    if animated_button(ui, "🔍 Scan Bluetooth", egui::Color32::from_rgb(49, 50, 68), egui::Color32::from_rgb(69, 71, 90), egui::Color32::from_rgb(205, 214, 244), 6.0, open_progress).clicked() {
                                        self.is_scanning = true;
                                        self.scanned_devices.clear();
                                        
                                        let tx_clone = self.status_tx.clone();
                                        let ctx_clone = ui.ctx().clone();
                                        tokio::spawn(async move {
                                            if let Ok(res) = reqwest::get("http://localhost:5050/api/bluetooth/scan").await {
                                                #[derive(Deserialize)]
                                                struct ScanRes { devices: Vec<BluetoothDevice> }
                                                if let Ok(scan) = res.json::<ScanRes>().await {
                                                    let bt_json = serde_json::json!({
                                                        "bluetooth_devices": scan.devices
                                                    }).to_string();
                                                    let _ = tx_clone.send(bt_json);
                                                }
                                            }
                                            let _ = tx_clone.send("ScanFinished".to_string());
                                            ctx_clone.request_repaint();
                                        });
                                    }
                                }
                                
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new("Devices:").size(11.0).strong());
                                ui.add_space(2.0);
                                
                                egui::ScrollArea::vertical()
                                    .id_source("bt_devices_scroll")
                                    .max_height(ui.available_height() - 10.0)
                                    .show(ui, |ui| {
                                        if self.scanned_devices.is_empty() {
                                            ui.label(egui::RichText::new("No devices found").size(10.0).italics().color(egui::Color32::from_rgb(108, 112, 134)));
                                        } else {
                                            for dev in &self.scanned_devices {
                                                let name = if dev.name.is_empty() { &dev.mac } else { &dev.name };
                                                let is_selected = self.phone_mac == dev.mac;
                                                let bg = if is_selected { egui::Color32::from_rgb(137, 180, 250) } else { egui::Color32::from_rgb(49, 50, 68) };
                                                let fg = if is_selected { egui::Color32::from_rgb(17, 17, 27) } else { egui::Color32::from_rgb(205, 214, 244) };
                                                
                                                if animated_button(ui, &format!("📱 {}", name), bg, egui::Color32::from_rgb(69, 71, 90), fg, 4.0, open_progress).clicked() {
                                                    self.phone_mac = dev.mac.clone();
                                                    self.save_bluetooth_settings();
                                                }
                                                if !dev.name.is_empty() {
                                                    ui.label(egui::RichText::new(&dev.mac).size(9.0).color(egui::Color32::from_rgb(108, 112, 134)));
                                                }
                                                ui.add_space(2.0);
                                            }
                                        }
                                    });
                            }
                        );
                        
                        // Separator line
                        let (sep_rect, _) = ui.allocate_exact_size(egui::vec2(10.0, ui.available_height()), egui::Sense::hover());
                        ui.painter().vline(sep_rect.center().x, sep_rect.y_range(), egui::Stroke::new(1.0, egui::Color32::from_rgb(49, 50, 68)));
                    }

                    // Right main area
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), ui.available_height()),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            // Recording Status banner
                            if self.is_recording {
                                let time = ui.input(|i| i.time);
                                let pulse = (time * 5.0).sin() as f32 * 0.3 + 0.7;
                                let dot_color = multiply_alpha(egui::Color32::from_rgb(243, 139, 168), pulse * open_progress);
                                let dot_glow = multiply_alpha(egui::Color32::from_rgb(243, 139, 168), open_progress * 0.15);
                                
                                let banner_bg_base = lerp_color(
                                    egui::Color32::from_rgb(80, 20, 30),
                                    egui::Color32::from_rgb(120, 30, 45),
                                    (time * 2.0).sin() as f32 * 0.5 + 0.5
                                );
                                let banner_bg = multiply_alpha(banner_bg_base, open_progress * 0.4);
                                
                                egui::Frame::none()
                                    .fill(banner_bg)
                                    .rounding(6.0)
                                    .inner_margin(8.0)
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            let (dot_rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                                            ui.painter().circle_filled(dot_rect.center(), 4.0, dot_color);
                                            ui.painter().circle_filled(dot_rect.center(), 7.0, dot_glow);
                                            ui.add_space(4.0);
                                            ui.label(egui::RichText::new(format!(
                                                "Recording Macro: '{}' ... Perform actions now, then tell AI to stop.",
                                                self.current_macro_name
                                            )).color(multiply_alpha(egui::Color32::from_rgb(243, 139, 168), open_progress)).strong().size(12.0));
                                        });
                                    });
                                ui.add_space(6.0);
                            }
                            
                            // Check for Spotlight math and app launches matches
                            let math_match = if !self.prompt.trim().is_empty() {
                                eval_math(&self.prompt)
                            } else {
                                None
                            };
                            
                            let mut matching_apps = Vec::new();
                            if !self.prompt.trim().is_empty() {
                                let query = self.prompt.trim().to_lowercase();
                                for app in APPS {
                                    if app.executable.to_lowercase().contains(&query) || app.name.to_lowercase().contains(&query) {
                                        matching_apps.push(app);
                                    }
                                }
                            }
                            
                            let input_height = 42.0;
                            let suggestions_height = if math_match.is_some() || !matching_apps.is_empty() {
                                let items_count = (if math_match.is_some() { 1 } else { 0 }) + matching_apps.len();
                                (items_count as f32 * 32.0) + 16.0
                            } else {
                                0.0
                            };
                            
                            let body_height = ui.available_height() - input_height - suggestions_height - 16.0;

                            // Chat area
                            egui::ScrollArea::vertical()
                                .max_height(body_height)
                                .auto_shrink([false; 2])
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    for msg in &self.messages {
                                        let elapsed = msg.added_at.map(|t| t.elapsed().as_secs_f32()).unwrap_or(1.0);
                                        let duration = 0.35;
                                        let progress = if elapsed < duration {
                                            ctx.request_repaint();
                                            let t = elapsed / duration;
                                            1.0 - (1.0 - t).powi(3) // Cubic ease out
                                        } else {
                                            1.0
                                        };

                                        let combined_opacity = progress * open_progress;

                                        let (bg_color, fg_color, align) = if msg.sender == "User" {
                                            (
                                                egui::Color32::from_rgb(137, 180, 250),
                                                egui::Color32::from_rgb(255, 255, 255),
                                                egui::Align::RIGHT,
                                            )
                                        } else {
                                            (
                                                egui::Color32::from_rgb(49, 50, 68),
                                                egui::Color32::from_rgb(205, 214, 244),
                                                egui::Align::LEFT,
                                            )
                                        };

                                        ui.with_layout(egui::Layout::top_down(align), |ui| {
                                            let vertical_offset = (1.0 - progress) * 20.0;
                                            ui.add_space(vertical_offset);

                                            let label = egui::RichText::new(&msg.content).color(multiply_alpha(fg_color, combined_opacity)).size(14.0);
                                            
                                            if msg.sender == "User" {
                                                let text_style = egui::TextStyle::Body;
                                                let wrap_width = ui.available_width() * 0.75;
                                                let text_job = egui::WidgetText::from(label).into_galley(ui, None, wrap_width, text_style);
                                                let padding = egui::vec2(12.0, 10.0);
                                                let bubble_size = text_job.size() + padding * 2.0;
                                                
                                                let (rect, _) = ui.allocate_exact_size(bubble_size, egui::Sense::hover());
                                                
                                                let painter = ui.painter();
                                                let top_c = multiply_alpha(egui::Color32::from_rgb(203, 166, 247), combined_opacity); // Mauve
                                                let bottom_c = multiply_alpha(egui::Color32::from_rgb(137, 180, 250), combined_opacity); // Blue
                                                
                                                paint_rounded_gradient(painter, rect, top_c, bottom_c, 14.0);
                                                painter.galley(rect.min + padding, text_job, multiply_alpha(egui::Color32::WHITE, combined_opacity));
                                            } else {
                                                let text_style = egui::TextStyle::Body;
                                                let wrap_width = ui.available_width() * 0.75;
                                                let text_job = egui::WidgetText::from(label).into_galley(ui, None, wrap_width, text_style);
                                                let padding = egui::vec2(12.0, 10.0);
                                                let bubble_size = text_job.size() + padding * 2.0;
                                                
                                                let (rect, _) = ui.allocate_exact_size(bubble_size, egui::Sense::hover());
                                                
                                                let painter = ui.painter();
                                                let rounding = egui::Rounding {
                                                    nw: 14.0,
                                                    ne: 14.0,
                                                    sw: 2.0,
                                                    se: 14.0,
                                                };
                                                
                                                painter.rect_filled(rect, rounding, multiply_alpha(bg_color, combined_opacity * 0.7));
                                                
                                                let border_stroke = egui::Stroke::new(1.0, multiply_alpha(egui::Color32::from_rgb(180, 190, 254), combined_opacity * 0.15));
                                                painter.rect_stroke(rect, rounding, border_stroke);
                                                
                                                painter.galley(rect.min + padding, text_job, multiply_alpha(fg_color, combined_opacity));
                                            }
                                        });
                                        ui.add_space(8.0);
                                    }

                                    if self.is_waiting {
                                        ui.horizontal(|ui| {
                                            paint_bouncing_dots(ui, 0.0, open_progress);
                                            ui.add_space(6.0);
                                            ui.label(egui::RichText::new("Aura is thinking & running commands...").color(multiply_alpha(egui::Color32::from_rgb(180, 190, 254), open_progress)).italics());
                                        });
                                    }
                                });

                            ui.separator();

                            // Autocomplete suggestion dropdown (Spotlight feature)
                            if math_match.is_some() || !matching_apps.is_empty() {
                                let border_color = multiply_alpha(egui::Color32::from_rgb(203, 166, 247), open_progress * 0.3);
                                let bg_color = multiply_alpha(egui::Color32::from_rgb(30, 30, 46), open_progress * 0.9);
                                
                                egui::Frame::none()
                                    .fill(bg_color)
                                    .rounding(8.0)
                                    .stroke(egui::Stroke::new(1.0, border_color))
                                    .inner_margin(8.0)
                                    .show(ui, |ui| {
                                        ui.vertical(|ui| {
                                            if let Some(res) = math_match {
                                                if animated_button(ui, &format!("🔢 Calculate: = {}", res), egui::Color32::TRANSPARENT, egui::Color32::from_rgb(69, 71, 90), egui::Color32::from_rgb(166, 227, 161), 6.0, open_progress).clicked() {
                                                    self.send_chat(ctx);
                                                }
                                            }
                                            for app in &matching_apps {
                                                if animated_button(ui, &format!("{} Launch {}", app.icon, app.name), egui::Color32::TRANSPARENT, egui::Color32::from_rgb(69, 71, 90), egui::Color32::from_rgb(205, 214, 244), 6.0, open_progress).clicked() {
                                                    let _ = std::process::Command::new(app.executable).spawn();
                                                    self.messages.push(Message {
                                                        sender: "User".to_string(),
                                                        content: format!("Launch {}", app.name),
                                                        added_at: Some(Instant::now()),
                                                    });
                                                    self.messages.push(Message {
                                                        sender: "Aura".to_string(),
                                                        content: format!("Launched {} ({})", app.name, app.executable),
                                                        added_at: Some(Instant::now()),
                                                    });
                                                    self.prompt.clear();
                                                }
                                            }
                                        });
                                    });
                                ui.add_space(6.0);
                            }

                            // Text input area
                            ui.horizontal(|ui| {
                                let text_edit = egui::TextEdit::singleline(&mut self.prompt)
                                    .hint_text("Ask Aura to do something... (e.g. 'update my system', 'record macro firefox')")
                                    .desired_width(ui.available_width() - 75.0)
                                    .margin(egui::vec2(8.0, 8.0))
                                    .frame(false)
                                    .text_color(multiply_alpha(egui::Color32::from_rgb(205, 214, 244), open_progress));

                                let response = ui.add(text_edit);
                                let painter = ui.painter();
                                
                                // Draw custom input container background
                                let input_bg = multiply_alpha(egui::Color32::from_rgb(30, 30, 46), open_progress * 0.8);
                                let input_border = multiply_alpha(egui::Color32::from_rgb(69, 71, 90), open_progress * 0.5);
                                painter.rect_filled(response.rect, 6.0, input_bg);
                                painter.rect_stroke(response.rect, 6.0, egui::Stroke::new(1.0, input_border));
                                
                                let focus_t = ui.ctx().animate_bool(response.id.with("focus_glow"), response.has_focus());
                                if focus_t > 0.0 {
                                    let glow_color = multiply_alpha(egui::Color32::from_rgb(203, 166, 247), focus_t * open_progress * 0.3);
                                    let glow_rect = response.rect.expand(1.5);
                                    painter.rect_stroke(glow_rect, 7.5, egui::Stroke::new(1.2 * focus_t, glow_color));
                                }
                                
                                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                    self.send_chat(ctx);
                                }
                                
                                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                    self.visible.store(false, Ordering::Relaxed);
                                }

                                if animated_button(
                                    ui, 
                                    "Send", 
                                    egui::Color32::from_rgb(180, 190, 254),
                                    egui::Color32::from_rgb(203, 166, 247),
                                    egui::Color32::from_rgb(17, 17, 27),
                                    6.0,
                                    open_progress
                                ).clicked() {
                                    self.send_chat(ctx);
                                }
                            });
                        }
                    );
                });
            }
        });
    }
}

#[tokio::main]
async fn main() -> Result<(), eframe::Error> {
    let args: Vec<String> = std::env::args().collect();

    // Toggle mode
    if args.len() > 1 && args[1] == "--toggle" {
        if let Ok(mut stream) = TcpStream::connect("127.0.0.1:5051").await {
            let _ = stream.write_all(b"toggle").await;
        }
        return Ok(());
    }

    // Set up single instance check & toggle listener
    let visible = Arc::new(AtomicBool::new(true));
    let visible_clone = visible.clone();

    tokio::spawn(async move {
        let listener = match TcpListener::bind("127.0.0.1:5051").await {
            Ok(l) => l,
            Err(_) => {
                // Already running, exit silently
                std::process::exit(0);
            }
        };

        loop {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0u8; 10];
                if let Ok(n) = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await {
                    if &buf[..n] == b"toggle" {
                        let cur = visible_clone.load(Ordering::Relaxed);
                        visible_clone.store(!cur, Ordering::Relaxed);
                    }
                }
            }
        }
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Aura OS AI")
            .with_inner_size([650.0, 480.0])
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_resizable(false),
        ..Default::default()
    };

    eframe::run_native(
        "Aura Client",
        options,
        Box::new(move |cc| Box::new(AuraApp::new(cc, visible))),
    )
}
