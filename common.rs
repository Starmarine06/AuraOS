use serde::{Deserialize, Serialize};

pub const SOCKET_PATH: &str = "/tmp/aura.sock";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PhoneConfig {
    pub mac_address: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RecordedEvent {
    pub delay_ms: u64,
    pub event_type: u16,
    pub code: u16,
    pub value: i32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatResponse {
    pub response: String,
    pub history: Vec<ChatMessage>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct StatusResponse {
    pub is_recording: bool,
    pub current_macro_name: String,
    pub model: String,
    pub available_macros: Vec<String>,
    pub phone_mac: String,
    pub bt_enabled: bool,
    pub is_connected: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BluetoothDevice {
    pub mac: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum IpcRequest {
    GetStatus,
    GetBluetoothStatus,
    ScanBluetooth,
    SetupBluetooth { mac_address: Option<String>, enabled: bool },
    SendChat { prompt: String },
    RecordMacro { name: String },
    StopRecordMacro,
    PlayMacro { name: String },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum IpcResponse {
    Status(StatusResponse),
    BluetoothStatus { mac: String, enabled: bool, is_connected: bool },
    BluetoothScan { devices: Vec<BluetoothDevice> },
    SetupBluetoothResult { success: bool },
    Chat(ChatResponse),
    MacroRecordResult { success: bool, message: String },
    MacroPlayResult { success: bool, message: String },
    Error(String),
}
