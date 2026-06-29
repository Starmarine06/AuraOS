use axum::{
    extract::{State, Json},
    routing::{post, get},
    Router,
};
use tower_http::cors::CorsLayer;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Instant, Duration};
use std::path::{Path, PathBuf};
use std::fs;
use std::process::Command;
use evdev::{
    uinput::{VirtualDevice, VirtualDeviceBuilder},
    AttributeSet, Key, RelativeAxisType, Device, InputEvent,
};
use futures_util::stream::StreamExt;
use reqwest::Client;
use tracing::{info, error, warn};

#[derive(Debug, Serialize, Deserialize, Clone)]
struct PhoneConfig {
    mac_address: Option<String>,
    enabled: bool,
}

#[derive(Clone)]
struct AppState {
    is_recording: Arc<Mutex<bool>>,
    current_macro_name: Arc<Mutex<String>>,
    recorded_events: Arc<Mutex<Vec<RecordedEvent>>>,
    virtual_device: Arc<tokio::sync::Mutex<Option<VirtualDevice>>>,
    macros_dir: PathBuf,
    last_event_time: Arc<Mutex<Option<Instant>>>,
    ollama_client: Client,
    ollama_model: Arc<Mutex<String>>,
    phone_mac: Arc<Mutex<Option<String>>>,
    proximity_lock_enabled: Arc<Mutex<bool>>,
    phone_config_path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct RecordedEvent {
    delay_ms: u64,
    event_type: u16,
    code: u16,
    value: i32,
}

#[derive(Debug, Deserialize)]
struct ChatRequest {
    prompt: String,
}

#[derive(Debug, Serialize)]
struct ChatResponse {
    response: String,
    history: Vec<OllamaMessage>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    format: String,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    message: OllamaMessage,
}

#[derive(Debug, Serialize, Deserialize)]
struct AgentAction {
    thought: String,
    action: String, // "execute", "record_macro", "play_macro", "respond"
    #[serde(default)]
    command: String,
    #[serde(default)]
    macro_name: String,
    #[serde(default)]
    response: String,
}

#[derive(Debug, Deserialize)]
struct RecordRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
struct PlayRequest {
    name: String,
}

#[derive(Debug, Serialize)]
struct StatusResponse {
    is_recording: bool,
    current_macro_name: String,
    model: String,
    available_macros: Vec<String>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let macros_dir = PathBuf::from("/var/lib/aura/macros");
    if let Err(e) = fs::create_dir_all(&macros_dir) {
        error!("Failed to create macros directory: {}", e);
    }

    // Initialize Virtual Device for uinput playback
    let vdev = match create_virtual_device() {
        Ok(d) => {
            info!("Successfully initialized virtual input device");
            Some(d)
        }
        Err(e) => {
            error!("Failed to initialize virtual input device: {}. Macro playback will not work.", e);
            None
        }
    };

    let phone_config_path = PathBuf::from("/var/lib/aura/phone_config.json");
    let mut mac = None;
    let mut enabled = false;
    if let Ok(content) = fs::read_to_string(&phone_config_path) {
        if let Ok(cfg) = serde_json::from_str::<PhoneConfig>(&content) {
            mac = cfg.mac_address;
            enabled = cfg.enabled;
        }
    }

    let state = AppState {
        is_recording: Arc::new(Mutex::new(false)),
        current_macro_name: Arc::new(Mutex::new(String::new())),
        recorded_events: Arc::new(Mutex::new(Vec::new())),
        virtual_device: Arc::new(tokio::sync::Mutex::new(vdev)),
        macros_dir,
        last_event_time: Arc::new(Mutex::new(None)),
        ollama_client: Client::new(),
        ollama_model: Arc::new(Mutex::new("qwen2.5:1.5b".to_string())),
        phone_mac: Arc::new(Mutex::new(mac)),
        proximity_lock_enabled: Arc::new(Mutex::new(enabled)),
        phone_config_path,
    };

    // Try to auto-detect model from Ollama
    let state_clone = state.clone();
    tokio::spawn(async move {
        detect_ollama_model(state_clone).await;
    });

    // Spawn background Bluetooth proximity monitor
    let state_monitor = state.clone();
    tokio::spawn(async move {
        let mut was_connected = true; // Assume connected initially to avoid false positives at startup
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;

            let mac = state_monitor.phone_mac.lock().unwrap().clone();
            let enabled = *state_monitor.proximity_lock_enabled.lock().unwrap();

            if enabled {
                if let Some(mac_addr) = mac {
                    // Ping Bluetooth device. l2ping exit code 0 means device is reachable.
                    let output = Command::new("l2ping")
                        .arg("-c")
                        .arg("1")
                        .arg("-t")
                        .arg("3")
                        .arg(&mac_addr)
                        .output();

                    let is_connected = match output {
                        Ok(out) => out.status.success(),
                        Err(_) => false,
                    };

                    info!("Proximity check for {}: connected={}", mac_addr, is_connected);

                    if was_connected && !is_connected {
                        // Phone just disconnected! Lock the XFCE session for user 'aura'
                        warn!("Phone disconnected. Locking screen!");
                        let lock_res = Command::new("sudo")
                            .arg("-u")
                            .arg("aura")
                            .arg("DISPLAY=:0.0")
                            .arg("XAUTHORITY=/home/aura/.Xauthority")
                            .arg("xflock4")
                            .output();
                        if let Err(e) = lock_res {
                            error!("Failed to lock screen: {}", e);
                        }
                    }
                    was_connected = is_connected;
                }
            } else {
                was_connected = true; // reset state
            }
        }
    });

    let app = Router::new()
        .route("/api/chat", post(handle_chat))
        .route("/api/macro/record", post(handle_record_start))
        .route("/api/macro/stop", post(handle_record_stop))
        .route("/api/macro/play", post(handle_play))
        .route("/api/status", get(handle_status))
        .route("/api/bluetooth/scan", get(handle_bluetooth_scan))
        .route("/api/bluetooth/status", get(handle_bluetooth_status))
        .route("/api/bluetooth/setup", post(handle_bluetooth_setup))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:5050").await.unwrap();
    info!("Aura Daemon running on http://0.0.0.0:5050");
    axum::serve(listener, app).await.unwrap();
}

fn create_virtual_device() -> std::io::Result<VirtualDevice> {
    let mut keys = AttributeSet::<Key>::new();
    // Add standard keyboard keys (0..256)
    for i in 0..256 {
        keys.insert(Key(i as u16));
    }
    // Add mouse buttons (0x110..0x118)
    for i in 0x110..0x118 {
        keys.insert(Key(i as u16));
    }

    let mut rel_axes = AttributeSet::<RelativeAxisType>::new();
    rel_axes.insert(RelativeAxisType::REL_X);
    rel_axes.insert(RelativeAxisType::REL_Y);
    rel_axes.insert(RelativeAxisType::REL_WHEEL);
    rel_axes.insert(RelativeAxisType::REL_HWHEEL);

    VirtualDeviceBuilder::new()?
        .name("Aura Virtual input device")
        .with_keys(&keys)?
        .with_relative_axes(&rel_axes)?
        .build()
}

async fn detect_ollama_model(state: AppState) {
    info!("Scanning Ollama for available models...");
    let url = "http://localhost:11434/api/tags";
    
    // Retry detection for 30 seconds (in case Ollama is starting up)
    for _ in 0..10 {
        if let Ok(res) = state.ollama_client.get(url).send().await {
            #[derive(Deserialize)]
            struct ModelInfo { name: String }
            #[derive(Deserialize)]
            struct TagsResponse { models: Vec<ModelInfo> }

            if let Ok(tags) = res.json::<TagsResponse>().await {
                if !tags.models.is_empty() {
                    let first_model = tags.models[0].name.clone();
                    info!("Detected Ollama model: {}", first_model);
                    *state.ollama_model.lock().unwrap() = first_model;
                    return;
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    warn!("No model detected from Ollama. Defaulting to 'qwen2.5:1.5b'");
}

async fn handle_status(State(state): State<AppState>) -> Json<StatusResponse> {
    let available_macros = get_macros_list(&state.macros_dir);
    Json(StatusResponse {
        is_recording: *state.is_recording.lock().unwrap(),
        current_macro_name: state.current_macro_name.lock().unwrap().clone(),
        model: state.ollama_model.lock().unwrap().clone(),
        available_macros,
    })
}

fn get_macros_list(dir: &Path) -> Vec<String> {
    let mut list = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Some(ext) = entry.path().extension() {
                if ext == "json" {
                    if let Some(stem) = entry.path().file_stem() {
                        list.push(stem.to_string_lossy().into_owned());
                    }
                }
            }
        }
    }
    list
}

async fn handle_record_start(
    State(state): State<AppState>,
    Json(payload): Json<RecordRequest>,
) -> Json<serde_json::Value> {
    let mut is_rec = state.is_recording.lock().unwrap();
    if *is_rec {
        return Json(serde_json::json!({ "status": "error", "message": "Already recording" }));
    }

    *is_rec = true;
    *state.current_macro_name.lock().unwrap() = payload.name.clone();
    state.recorded_events.lock().unwrap().clear();
    *state.last_event_time.lock().unwrap() = Some(Instant::now());

    info!("Starting recording macro: {}", payload.name);

    // Spawn background device readers
    let state_clone = state.clone();
    tokio::spawn(async move {
        start_physical_capture(state_clone).await;
    });

    Json(serde_json::json!({ "status": "success", "message": format!("Recording macro '{}' started", payload.name) }))
}

async fn handle_record_stop(State(state): State<AppState>) -> Json<serde_json::Value> {
    let mut is_rec = state.is_recording.lock().unwrap();
    if !*is_rec {
        return Json(serde_json::json!({ "status": "error", "message": "Not recording" }));
    }

    *is_rec = false;
    let name = state.current_macro_name.lock().unwrap().clone();
    let events = state.recorded_events.lock().unwrap().clone();
    
    info!("Stopped recording macro: {}. Recorded {} events.", name, events.len());

    // Save macro
    let file_path = state.macros_dir.join(format!("{}.json", name));
    if let Ok(content) = serde_json::to_string(&events) {
        if let Err(e) = fs::write(&file_path, content) {
            error!("Failed to write macro file: {}", e);
            return Json(serde_json::json!({ "status": "error", "message": format!("Failed to save macro: {}", e) }));
        }
    } else {
        return Json(serde_json::json!({ "status": "error", "message": "Serialization failure" }));
    }

    Json(serde_json::json!({ "status": "success", "message": format!("Macro '{}' saved successfully", name) }))
}

async fn handle_play(
    State(state): State<AppState>,
    Json(payload): Json<PlayRequest>,
) -> Json<serde_json::Value> {
    info!("Playing macro: {}", payload.name);
    let file_path = state.macros_dir.join(format!("{}.json", payload.name));
    if !file_path.exists() {
        return Json(serde_json::json!({ "status": "error", "message": "Macro not found" }));
    }

    let content = match fs::read_to_string(&file_path) {
        Ok(c) => c,
        Err(e) => return Json(serde_json::json!({ "status": "error", "message": e.to_string() })),
    };

    let events: Vec<RecordedEvent> = match serde_json::from_str(&content) {
        Ok(evs) => evs,
        Err(e) => return Json(serde_json::json!({ "status": "error", "message": e.to_string() })),
    };

    let mut vdev_lock = state.virtual_device.lock().await;
    let vdev = match &mut *vdev_lock {
        Some(d) => d,
        None => return Json(serde_json::json!({ "status": "error", "message": "Virtual input device not available" })),
    };

    // Playback events
    for event in events {
        if event.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(event.delay_ms)).await;
        }

        let ev = InputEvent::new(evdev::EventType(event.event_type), event.code, event.value);
        if let Err(e) = vdev.emit(&[ev]) {
            error!("Failed to play event: {}", e);
        }
    }

    Json(serde_json::json!({ "status": "success", "message": format!("Macro '{}' played successfully", payload.name) }))
}

async fn start_physical_capture(state: AppState) {
    let dev_dir = Path::new("/dev/input");
    let mut devices = Vec::new();

    if let Ok(entries) = fs::read_dir(dev_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                if file_name.starts_with("event") {
                    if let Ok(device) = Device::open(&path) {
                        // Skip our virtual device to prevent feedback loop
                        if let Some(name) = device.name() {
                            if name.contains("Aura Virtual") {
                                continue;
                            }
                        }
                        devices.push(device);
                    }
                }
            }
        }
    }

    info!("Recording from {} physical input devices", devices.len());

    let mut streams = Vec::new();
    for dev in devices {
        if let Ok(stream) = dev.into_event_stream() {
            streams.push(stream);
        }
    }

    if streams.is_empty() {
        warn!("No input devices captured!");
        return;
    }

    // Combine streams and record
    let mut event_futures = futures_util::stream::select_all(streams);
    while *state.is_recording.lock().unwrap() {
        tokio::select! {
            Some(ev_res) = event_futures.next() => {
                if let Ok(ev) = ev_res {
                    // Check if still recording
                    if !*state.is_recording.lock().unwrap() {
                        break;
                    }

                    let mut last_time_lock = state.last_event_time.lock().unwrap();
                    let delay_ms = match *last_time_lock {
                        Some(last) => last.elapsed().as_millis() as u64,
                        None => 0,
                    };
                    *last_time_lock = Some(Instant::now());

                    let recorded = RecordedEvent {
                        delay_ms,
                        event_type: ev.event_type().0,
                        code: ev.code(),
                        value: ev.value(),
                    };
                    state.recorded_events.lock().unwrap().push(recorded);
                }
            }
            else => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }
}

async fn handle_chat(
    State(state): State<AppState>,
    Json(payload): Json<ChatRequest>,
) -> Json<ChatResponse> {
    let mut history = vec![
        OllamaMessage {
            role: "system".to_string(),
            content: get_system_prompt(),
        },
        OllamaMessage {
            role: "user".to_string(),
            content: payload.prompt,
        }
    ];

    let mut iteration = 0;
    let max_iterations = 8;
    let model = state.ollama_model.lock().unwrap().clone();

    loop {
        iteration += 1;
        if iteration > max_iterations {
            return Json(ChatResponse {
                response: "Agent loop exceeded maximum execution steps.".to_string(),
                history,
            });
        }

        // Call Ollama
        let ollama_req = OllamaChatRequest {
            model: model.clone(),
            messages: history.clone(),
            format: "json".to_string(),
            stream: false,
        };

        let res = match state.ollama_client
            .post("http://localhost:11434/api/chat")
            .json(&ollama_req)
            .send()
            .await 
        {
            Ok(r) => r,
            Err(e) => {
                return Json(ChatResponse {
                    response: format!("Error communicating with local Ollama service: {}", e),
                    history,
                });
            }
        };

        let ollama_res = match res.json::<OllamaChatResponse>().await {
            Ok(r) => r,
            Err(e) => {
                return Json(ChatResponse {
                    response: format!("Failed to parse JSON response from Ollama: {}", e),
                    history,
                });
            }
        };

        let agent_message = ollama_res.message.content.clone();
        history.push(ollama_res.message);

        let action: AgentAction = match serde_json::from_str(&agent_message) {
            Ok(act) => act,
            Err(e) => {
                error!("Model output invalid JSON: {}. Response content: {}", e, agent_message);
                return Json(ChatResponse {
                    response: format!("AI hallucinated invalid JSON format. Raw output: {}", agent_message),
                    history,
                });
            }
        };

        info!("Agent thought: {}", action.thought);

        match action.action.as_str() {
            "respond" => {
                return Json(ChatResponse {
                    response: action.response,
                    history,
                });
            }
            "execute" => {
                info!("Agent requested execution: {}", action.command);
                let output = run_shell_command(&action.command).await;
                info!("Execution output: {}", output);
                history.push(OllamaMessage {
                    role: "user".to_string(),
                    content: format!(
                        "{{\"action_result\": \"executed\", \"command\": {:?}, \"output\": {:?}}}",
                        action.command, output
                    ),
                });
            }
            "record_macro" => {
                info!("Agent requested recording macro: {}", action.macro_name);
                let result = trigger_record_start(&state, &action.macro_name).await;
                history.push(OllamaMessage {
                    role: "user".to_string(),
                    content: format!(
                        "{{\"action_result\": \"record_macro_triggered\", \"macro_name\": {:?}, \"result\": {:?}}}",
                        action.macro_name, result
                    ),
                });
            }
            "play_macro" => {
                info!("Agent requested playing macro: {}", action.macro_name);
                let result = trigger_play(&state, &action.macro_name).await;
                history.push(OllamaMessage {
                    role: "user".to_string(),
                    content: format!(
                        "{{\"action_result\": \"play_macro_completed\", \"macro_name\": {:?}, \"result\": {:?}}}",
                        action.macro_name, result
                    ),
                });
            }
            other => {
                warn!("Unknown action: {}", other);
                return Json(ChatResponse {
                    response: format!("Agent attempted unknown action: {}", other),
                    history,
                });
            }
        }
    }
}

async fn run_shell_command(cmd: &str) -> String {
    let output = Command::new("bash")
        .arg("-c")
        .arg(cmd)
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            format!("STDOUT:\n{}\nSTDERR:\n{}", stdout, stderr)
        }
        Err(e) => format!("Execution Error: {}", e),
    }
}

async fn trigger_record_start(state: &AppState, name: &str) -> String {
    let mut is_rec = state.is_recording.lock().unwrap();
    if *is_rec {
        return "Error: Already recording".to_string();
    }
    *is_rec = true;
    *state.current_macro_name.lock().unwrap() = name.to_string();
    state.recorded_events.lock().unwrap().clear();
    *state.last_event_time.lock().unwrap() = Some(Instant::now());

    let state_clone = state.clone();
    tokio::spawn(async move {
        start_physical_capture(state_clone).await;
    });

    format!("Success: Recording for macro '{}' started.", name)
}

async fn trigger_play(state: &AppState, name: &str) -> String {
    let file_path = state.macros_dir.join(format!("{}.json", name));
    if !file_path.exists() {
        return format!("Error: Macro '{}' not found", name);
    }
    let content = match fs::read_to_string(&file_path) {
        Ok(c) => c,
        Err(e) => return format!("Error reading macro: {}", e),
    };
    let events: Vec<RecordedEvent> = match serde_json::from_str(&content) {
        Ok(evs) => evs,
        Err(e) => return format!("Error parsing macro: {}", e),
    };
    let mut vdev_lock = state.virtual_device.lock().await;
    let vdev = match &mut *vdev_lock {
        Some(d) => d,
        None => return "Error: Virtual input device not available".to_string(),
    };

    for event in events {
        if event.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(event.delay_ms)).await;
        }
        let ev = InputEvent::new(evdev::EventType(event.event_type), event.code, event.value);
        let _ = vdev.emit(&[ev]);
    }
    format!("Success: Macro '{}' played back successfully", name)
}

fn get_system_prompt() -> String {
    r#"You are AuraOS Agent, a built-in AI assistant running inside a custom Linux distribution. You run with ROOT privileges.
You have the power to run shell commands, record input macros, play back macros, and assist the user.
You MUST output your responses in the following JSON format ONLY:
{
  "thought": "Your internal thinking process outlining what you need to do next",
  "action": "execute" | "record_macro" | "play_macro" | "respond",
  "command": "The shell command to run (required ONLY if action is 'execute')",
  "macro_name": "The macro name (required ONLY if action is 'record_macro' or 'play_macro')",
  "response": "The final message explaining what you did, your findings, or answering the user (required ONLY if action is 'respond')"
}

Guidelines:
1. Use "execute" to run standard Linux bash commands (e.g., install packages using pacman, edit configurations, start services, query status).
2. Use "record_macro" to start recording physical keyboard and mouse events. The user will perform the action after you prompt them. Make sure to respond and instruct them after launching the record.
3. Use "play_macro" to replay a recorded macro.
4. Use "respond" when you have finished your tasks or want to answer a question.
5. You operate in an Agentic Loop: if you use "execute", "record_macro", or "play_macro", the system will run the action and feed the result back into your context as a new message, allowing you to react and run more commands if needed.
6. Only output valid JSON. Do not add markdown quotes around the JSON, just print the JSON directly."#.to_string()
}

async fn handle_bluetooth_scan(State(_state): State<AppState>) -> Json<serde_json::Value> {
    info!("Starting Bluetooth active scan...");
    // Trigger active scan in background to populate devices
    let _ = Command::new("bluetoothctl")
        .arg("--timeout")
        .arg("3")
        .arg("scan")
        .arg("on")
        .output();

    let output = Command::new("bluetoothctl")
        .arg("devices")
        .output();
        
    let mut list = Vec::new();
    if let Ok(out) = output {
        let stdout = String::from_utf8_lossy(&out.stdout);
        for line in stdout.lines() {
            // bluetoothctl devices format: Device 00:11:22:33:44:55 Device Name
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 && parts[0] == "Device" {
                let mac = parts[1].to_string();
                let name = parts[2..].join(" ");
                list.push(serde_json::json!({
                    "mac": mac,
                    "name": name
                }));
            }
        }
    }
    Json(serde_json::json!({ "devices": list }))
}

#[derive(Debug, Serialize)]
struct BluetoothStatusResponse {
    mac_address: Option<String>,
    enabled: bool,
    is_connected: bool,
}

async fn handle_bluetooth_status(State(state): State<AppState>) -> Json<BluetoothStatusResponse> {
    let mac = state.phone_mac.lock().unwrap().clone();
    let enabled = *state.proximity_lock_enabled.lock().unwrap();
    let mut is_connected = false;

    if let Some(ref mac_addr) = mac {
        let output = Command::new("l2ping")
            .arg("-c")
            .arg("1")
            .arg("-t")
            .arg("1")
            .arg(mac_addr)
            .output();
        if let Ok(out) = output {
            is_connected = out.status.success();
        }
    }

    Json(BluetoothStatusResponse {
        mac_address: mac,
        enabled,
        is_connected,
    })
}

#[derive(Debug, Deserialize)]
struct BluetoothSetupRequest {
    mac_address: Option<String>,
    enabled: bool,
}

async fn handle_bluetooth_setup(
    State(state): State<AppState>,
    Json(payload): Json<BluetoothSetupRequest>,
) -> Json<serde_json::Value> {
    *state.phone_mac.lock().unwrap() = payload.mac_address.clone();
    *state.proximity_lock_enabled.lock().unwrap() = payload.enabled;

    let cfg = PhoneConfig {
        mac_address: payload.mac_address,
        enabled: payload.enabled,
    };

    if let Ok(content) = serde_json::to_string(&cfg) {
        if let Err(e) = fs::write(&state.phone_config_path, content) {
            error!("Failed to write phone config file: {}", e);
        }
    }

    Json(serde_json::json!({ "status": "success", "message": "Bluetooth configuration updated" }))
}
