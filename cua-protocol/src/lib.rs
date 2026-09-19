//! Strict, versioned, window-only protocol for adapting Cua Driver 0.28.x.
//!
//! This crate is deliberately transport-free. A [`MachineId`] is only a stable
//! selector; a future transport must authenticate and bind it to a connection.
//! V1 admits background delivery only and cannot represent desktop scope,
//! foreground escalation, arbitrary command arguments, URLs, or file paths.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use serde::{Deserialize, Deserializer, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fmt,
    future::Future,
    num::NonZeroU32,
    pin::Pin,
    sync::Arc,
};

pub const MAX_ID_BYTES: usize = 128;
pub const MAX_TEXT_BYTES: usize = 16 * 1024;
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
pub const MAX_REGISTRATION_BYTES: usize = 16 * 1024;
pub const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_JSON_DEPTH: usize = 16;
pub const MAX_ELEMENTS: usize = 2_000;
pub const MAX_WINDOWS: usize = 1_024;
pub const MAX_APPS: usize = 1_024;
pub const MAX_SESSIONS: usize = 100;
pub const MAX_MODIFIERS: usize = 5;
pub const MAX_MENU_DEPTH: usize = 16;
pub const MAX_VERIFY_PREDICATES: usize = 8;
pub const MAX_DECODED_IMAGE_BYTES: usize = 6 * 1024 * 1024;
pub const MAX_HEALTH_DATA_FIELDS: usize = 32;
pub const MAX_HEALTH_DATA_ITEMS: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationError(String);

impl ValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}
impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ValidationError {}

fn validate_identifier(name: &str, value: &str, max: usize) -> Result<(), ValidationError> {
    if value.is_empty() || value.len() > max {
        return Err(ValidationError::new(format!("invalid {name} length")));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(ValidationError::new(format!("invalid {name} grammar")));
    }
    Ok(())
}

fn validate_text(name: &str, value: &str, max: usize) -> Result<(), ValidationError> {
    if value.is_empty() || value.len() > max {
        return Err(ValidationError::new(format!("invalid {name} length")));
    }
    validate_empty_text(name, value, max)
}

fn validate_empty_text(name: &str, value: &str, max: usize) -> Result<(), ValidationError> {
    if value.len() > max {
        return Err(ValidationError::new(format!("invalid {name} length")));
    }
    if value
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(ValidationError::new(format!(
            "invalid {name} control character"
        )));
    }
    Ok(())
}

fn validate_output_path(name: &str, value: &str, max: usize) -> Result<(), ValidationError> {
    validate_single_line(name, value, max)?;
    if value.contains('\0') {
        return Err(ValidationError::new(format!("invalid {name}")));
    }
    Ok(())
}

fn validate_timestamp(name: &str, value: &str, max: usize) -> Result<(), ValidationError> {
    validate_single_line(name, value, max)?;
    let bytes = value.as_bytes();
    let fixed = bytes.len() >= 20
        && bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && bytes.get(10) == Some(&b'T')
        && bytes.get(13) == Some(&b':')
        && bytes.get(16) == Some(&b':')
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..10].iter().all(u8::is_ascii_digit)
        && bytes[11..13].iter().all(u8::is_ascii_digit)
        && bytes[14..16].iter().all(u8::is_ascii_digit)
        && bytes[17..19].iter().all(u8::is_ascii_digit);
    let month = value.get(5..7).and_then(|part| part.parse::<u8>().ok());
    let day = value.get(8..10).and_then(|part| part.parse::<u8>().ok());
    let hour = value.get(11..13).and_then(|part| part.parse::<u8>().ok());
    let minute = value.get(14..16).and_then(|part| part.parse::<u8>().ok());
    let second = value.get(17..19).and_then(|part| part.parse::<u8>().ok());
    let timezone = value.ends_with('Z')
        || value
            .get(19..)
            .is_some_and(|suffix| suffix.len() == 6 && matches!(suffix.as_bytes()[0], b'+' | b'-'));
    if fixed
        && month.is_some_and(|part| (1..=12).contains(&part))
        && day.is_some_and(|part| (1..=31).contains(&part))
        && hour.is_some_and(|part| part <= 23)
        && minute.is_some_and(|part| part <= 59)
        && second.is_some_and(|part| part <= 60)
        && timezone
    {
        Ok(())
    } else {
        Err(ValidationError::new(format!("invalid {name}")))
    }
}

fn validate_key_name(name: &str, value: &str, _: usize) -> Result<(), ValidationError> {
    let named = matches!(
        value,
        "return"
            | "tab"
            | "escape"
            | "up"
            | "down"
            | "left"
            | "right"
            | "space"
            | "delete"
            | "home"
            | "end"
            | "pageup"
            | "pagedown"
    );
    let function = value
        .strip_prefix('f')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=12).contains(&number));
    let scalar = value.len() == 1
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || byte.is_ascii_digit());
    if named || function || scalar {
        Ok(())
    } else {
        Err(ValidationError::new(format!("invalid {name}")))
    }
}

fn is_hotkey_modifier(value: &str) -> bool {
    matches!(value, "cmd" | "shift" | "option" | "ctrl" | "fn")
}

fn validate_hotkey_key(name: &str, value: &str, max: usize) -> Result<(), ValidationError> {
    if is_hotkey_modifier(value) {
        Ok(())
    } else {
        validate_key_name(name, value, max)
    }
}

fn validate_opaque_token(name: &str, value: &str, max: usize) -> Result<(), ValidationError> {
    if value.is_empty() || value.len() > max {
        return Err(ValidationError::new(format!("invalid {name} length")));
    }
    if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(ValidationError::new(format!("invalid {name} grammar")));
    }
    Ok(())
}

fn validate_single_line(name: &str, value: &str, max: usize) -> Result<(), ValidationError> {
    if value.is_empty() || value.len() > max || value.trim() != value {
        return Err(ValidationError::new(format!("invalid {name} length")));
    }
    if value.chars().any(char::is_control) {
        return Err(ValidationError::new(format!("invalid {name} grammar")));
    }
    Ok(())
}

fn validate_bundle_id(name: &str, value: &str, max: usize) -> Result<(), ValidationError> {
    if value.is_empty() || value.len() > max || !value.contains('.') {
        return Err(ValidationError::new(format!("invalid {name} length")));
    }
    if value.split('.').any(|part| {
        part.is_empty()
            || !part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || !part
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            || !part
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric)
    }) {
        return Err(ValidationError::new(format!("invalid {name} grammar")));
    }
    Ok(())
}

fn validate_health_check_name(name: &str, value: &str, max: usize) -> Result<(), ValidationError> {
    if value.is_empty()
        || value.len() > max
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(ValidationError::new(format!("invalid {name}")));
    }
    Ok(())
}

macro_rules! string_type {
    ($name:ident, $max:expr, $validator:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);
        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl TryFrom<&str> for $name {
            type Error = ValidationError;
            fn try_from(value: &str) -> Result<Self, Self::Error> {
                $validator(stringify!($name), value, $max)?;
                Ok(Self(value.to_owned()))
            }
        }
        impl TryFrom<String> for $name {
            type Error = ValidationError;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                $validator(stringify!($name), &value, $max)?;
                Ok(Self(value))
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::try_from(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

string_type!(MachineId, MAX_ID_BYTES, validate_identifier);
string_type!(RequestId, MAX_ID_BYTES, validate_identifier);
string_type!(SessionLabel, 64, validate_identifier);
string_type!(DriverVersion, 64, validate_identifier);
string_type!(SnapshotId, 16, validate_snapshot_id);
string_type!(ElementToken, 256, validate_opaque_token);
string_type!(BoundedText, MAX_TEXT_BYTES, validate_text);
string_type!(EmptyValueText, MAX_TEXT_BYTES, validate_empty_text);
string_type!(EmptyTitleText, 1_024, validate_empty_text);
string_type!(TreeMarkdown, MAX_TEXT_BYTES, validate_empty_text);
string_type!(OutputPath, 4_096, validate_output_path);
string_type!(Timestamp, 64, validate_timestamp);
string_type!(KeyName, 16, validate_key_name);
string_type!(HotkeyKey, 16, validate_hotkey_key);
string_type!(AppBundleId, 256, validate_bundle_id);
string_type!(AppDisplayName, 256, validate_single_line);
pub type AppIdentifier = AppBundleId;
string_type!(PageCursor, 256, validate_text);
string_type!(HealthCheckName, 64, validate_health_check_name);
string_type!(HealthDataKey, 64, validate_health_check_name);

fn validate_snapshot_id(name: &str, value: &str, _: usize) -> Result<(), ValidationError> {
    let valid = value.len() == 9
        && value.starts_with('s')
        && value[1..]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
    if valid {
        Ok(())
    } else {
        Err(ValidationError::new(format!("invalid {name}")))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolVersion {
    V1,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Macos,
    Windows,
    Linux,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineLocation {
    ServerLocal,
    Desktop,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineHealth {
    Healthy,
    Degraded,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Granted,
    Denied,
    PromptRequired,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionState {
    pub accessibility: Permission,
    pub screen_capture: Permission,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    Accessibility,
    ScreenCapture,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    AppDiscovery,
    AppLaunch,
    WindowDiscovery,
    WindowObservation,
    WindowManagement,
    Pointer,
    Keyboard,
    ElementValue,
    Menu,
    Verification,
    SessionLifecycle,
    Health,
}

impl Capability {
    fn descriptor_permissions(self) -> &'static [PermissionKind] {
        use PermissionKind::*;
        match self {
            Self::WindowObservation => &[],
            Self::WindowManagement
            | Self::Pointer
            | Self::Keyboard
            | Self::ElementValue
            | Self::Menu
            | Self::Verification => &[Accessibility],
            _ => &[],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MachineDescriptor {
    /// Stable selector only. A runtime transport must bind this to authenticated identity.
    pub machine_id: MachineId,
    pub location: MachineLocation,
    pub platform: Platform,
    pub driver_version: DriverVersion,
    pub health: MachineHealth,
    pub permissions: PermissionState,
    pub capabilities: Vec<Capability>,
}

impl MachineDescriptor {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.capabilities.len() > Capability::COUNT {
            return Err(ValidationError::new("too many capabilities"));
        }
        let mut seen = HashSet::new();
        for capability in &self.capabilities {
            if !seen.insert(*capability) {
                return Err(ValidationError::new("duplicate capability"));
            }
            for permission in capability.descriptor_permissions() {
                if self.permission(*permission) == Permission::Unavailable {
                    return Err(ValidationError::new(
                        "capability contradicts unavailable permission",
                    ));
                }
            }
            if *capability == Capability::WindowObservation
                && self.permissions.accessibility == Permission::Unavailable
                && self.permissions.screen_capture == Permission::Unavailable
            {
                return Err(ValidationError::new(
                    "window observation requires an available modality",
                ));
            }
        }
        Ok(())
    }

    pub fn authorize(&self, action: &CuaAction) -> Result<(), ValidationError> {
        self.validate()?;
        action.validate()?;
        if self.health == MachineHealth::Unavailable {
            return Err(ValidationError::new("machine is unavailable"));
        }
        if !self.capabilities.contains(&action.required_capability()) {
            return Err(ValidationError::new("capability not advertised"));
        }
        for permission in action.required_permissions() {
            if self.permission(permission) != Permission::Granted {
                return Err(ValidationError::new("required permission is not granted"));
            }
        }
        Ok(())
    }

    fn permission(&self, kind: PermissionKind) -> Permission {
        match kind {
            PermissionKind::Accessibility => self.permissions.accessibility,
            PermissionKind::ScreenCapture => self.permissions.screen_capture,
        }
    }
}

impl Capability {
    const COUNT: usize = 12;
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CuaRegistrationEnvelope {
    pub version: ProtocolVersion,
    pub machine: MachineDescriptor,
}
impl CuaRegistrationEnvelope {
    pub fn from_json(json: &str) -> Result<Self, ValidationError> {
        decode_limited(
            json,
            MAX_REGISTRATION_BYTES,
            "registration",
            |value: Self| {
                value.machine.validate()?;
                Ok(value)
            },
        )
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowTarget {
    pub pid: u32,
    pub window_id: u64,
}
impl Eq for WindowTarget {}
impl WindowTarget {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.pid == 0 || self.window_id == 0 {
            Err(ValidationError::new("pid and window_id must be positive"))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowPoint {
    pub x: f64,
    pub y: f64,
}
impl WindowPoint {
    fn validate(&self) -> Result<(), ValidationError> {
        finite(self.x, "x")?;
        finite(self.y, "y")
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Result<Self, ValidationError> {
        let value = Self {
            x,
            y,
            width,
            height,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), ValidationError> {
        finite(self.x, "x")?;
        finite(self.y, "y")?;
        finite(self.width, "width")?;
        finite(self.height, "height")?;
        if self.width <= 0.0 || self.height <= 0.0 {
            return Err(ValidationError::new("width and height must be positive"));
        }
        Ok(())
    }
}

fn finite(value: f64, name: &str) -> Result<(), ValidationError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(ValidationError::new(format!("{name} must be finite")))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ElementAddress {
    ElementToken {
        element_token: ElementToken,
    },
    ElementIndex {
        element_index: u32,
        snapshot_id: SnapshotId,
    },
    Point(WindowPoint),
}
impl ElementAddress {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Point(point) => point.validate(),
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ElementRef {
    ElementToken {
        element_token: ElementToken,
    },
    ElementIndex {
        element_index: u32,
        snapshot_id: SnapshotId,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    Background,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClickAction {
    Press,
    ShowMenu,
    Pick,
    Confirm,
    Cancel,
    Open,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Modifier {
    #[serde(rename = "cmd")]
    Command,
    Shift,
    Option,
    #[serde(rename = "ctrl")]
    Control,
    #[serde(rename = "fn")]
    Function,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollGranularity {
    Line,
    Page,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyArgs {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchAppArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<AppBundleId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<AppDisplayName>,
    pub creates_new_application_instance: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListWindowsArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<NonZeroU32>,
    pub on_screen_only: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GetWindowStateArgs {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub include_accessibility_tree: bool,
    pub include_screenshot: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_elements: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_dimension: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<BoundedText>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetWindowFrameArgs {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub frame: Rect,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClickArgs {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub delivery_mode: DeliveryMode,
    pub address: ElementAddress,
    pub button: MouseButton,
    pub action: ClickAction,
    pub modifiers: Vec<Modifier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u8>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AddressedActionArgs {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub delivery_mode: DeliveryMode,
    pub address: ElementAddress,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RightClickArgs {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub delivery_mode: DeliveryMode,
    pub address: ElementAddress,
    pub modifiers: Vec<Modifier>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MoveCursorArgs {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub delivery_mode: DeliveryMode,
    pub point: WindowPoint,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DragArgs {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub delivery_mode: DeliveryMode,
    pub from: WindowPoint,
    pub to: WindowPoint,
    pub duration_ms: u16,
    pub steps: u8,
    pub button: MouseButton,
    pub modifiers: Vec<Modifier>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScrollArgs {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub delivery_mode: DeliveryMode,
    pub address: ElementAddress,
    pub direction: ScrollDirection,
    pub by: ScrollGranularity,
    pub amount: u8,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TypeTextArgs {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub delivery_mode: DeliveryMode,
    pub address: ElementAddress,
    pub text: BoundedText,
    pub delay_ms: u8,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PressKeyArgs {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub delivery_mode: DeliveryMode,
    pub address: ElementAddress,
    pub key: KeyName,
    pub modifiers: Vec<Modifier>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HotkeyArgs {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub delivery_mode: DeliveryMode,
    pub address: ElementAddress,
    pub keys: Vec<HotkeyKey>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetValueArgs {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub element: ElementRef,
    pub value: EmptyValueText,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvokeMenuArgs {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub path: Vec<BoundedText>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    tag = "condition",
    content = "expected",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ElementCondition {
    Exists,
    Enabled(bool),
    Selected(bool),
    ValueEquals(EmptyValueText),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ElementSelector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<BoundedText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_contains: Option<BoundedText>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ElementPredicate {
    pub selector: ElementSelector,
    pub condition: ElementCondition,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    tag = "predicate",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum VerifyPredicate {
    WindowExists(bool),
    WindowBounds { bounds: Rect, tolerance_px: f64 },
    Element(ElementPredicate),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyStateArgs {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub expect: Vec<VerifyPredicate>,
    pub include_screenshot: bool,
    pub stable_samples: u8,
    pub timeout_ms: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StartSessionArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionRefArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListSessionsArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<PageCursor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HealthReportArgs {
    pub include: Vec<HealthCheckName>,
    pub skip: Vec<HealthCheckName>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    tag = "tool",
    content = "args",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum CuaAction {
    ListApps(EmptyArgs),
    LaunchApp(LaunchAppArgs),
    ListWindows(ListWindowsArgs),
    GetWindowState(GetWindowStateArgs),
    SetWindowFrame(SetWindowFrameArgs),
    Click(ClickArgs),
    DoubleClick(AddressedActionArgs),
    RightClick(RightClickArgs),
    MoveCursor(MoveCursorArgs),
    Drag(DragArgs),
    Scroll(ScrollArgs),
    TypeText(TypeTextArgs),
    PressKey(PressKeyArgs),
    Hotkey(HotkeyArgs),
    SetValue(SetValueArgs),
    InvokeMenu(InvokeMenuArgs),
    VerifyState(VerifyStateArgs),
    StartSession(StartSessionArgs),
    GetSession(SessionRefArgs),
    ListSessions(ListSessionsArgs),
    EndSession(SessionRefArgs),
    HealthReport(HealthReportArgs),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CuaActionKind {
    ListApps,
    LaunchApp,
    ListWindows,
    GetWindowState,
    SetWindowFrame,
    Click,
    DoubleClick,
    RightClick,
    MoveCursor,
    Drag,
    Scroll,
    TypeText,
    PressKey,
    Hotkey,
    SetValue,
    InvokeMenu,
    VerifyState,
    StartSession,
    GetSession,
    ListSessions,
    EndSession,
    HealthReport,
}
impl CuaActionKind {
    pub const ALL: [Self; 22] = [
        Self::ListApps,
        Self::LaunchApp,
        Self::ListWindows,
        Self::GetWindowState,
        Self::SetWindowFrame,
        Self::Click,
        Self::DoubleClick,
        Self::RightClick,
        Self::MoveCursor,
        Self::Drag,
        Self::Scroll,
        Self::TypeText,
        Self::PressKey,
        Self::Hotkey,
        Self::SetValue,
        Self::InvokeMenu,
        Self::VerifyState,
        Self::StartSession,
        Self::GetSession,
        Self::ListSessions,
        Self::EndSession,
        Self::HealthReport,
    ];
}

impl CuaAction {
    pub fn from_json(json: &str) -> Result<Self, ValidationError> {
        decode_limited(json, MAX_REQUEST_BYTES, "action", |value: Self| {
            value.validate()?;
            Ok(value)
        })
    }
    pub const fn kind(&self) -> CuaActionKind {
        match self {
            Self::ListApps(_) => CuaActionKind::ListApps,
            Self::LaunchApp(_) => CuaActionKind::LaunchApp,
            Self::ListWindows(_) => CuaActionKind::ListWindows,
            Self::GetWindowState(_) => CuaActionKind::GetWindowState,
            Self::SetWindowFrame(_) => CuaActionKind::SetWindowFrame,
            Self::Click(_) => CuaActionKind::Click,
            Self::DoubleClick(_) => CuaActionKind::DoubleClick,
            Self::RightClick(_) => CuaActionKind::RightClick,
            Self::MoveCursor(_) => CuaActionKind::MoveCursor,
            Self::Drag(_) => CuaActionKind::Drag,
            Self::Scroll(_) => CuaActionKind::Scroll,
            Self::TypeText(_) => CuaActionKind::TypeText,
            Self::PressKey(_) => CuaActionKind::PressKey,
            Self::Hotkey(_) => CuaActionKind::Hotkey,
            Self::SetValue(_) => CuaActionKind::SetValue,
            Self::InvokeMenu(_) => CuaActionKind::InvokeMenu,
            Self::VerifyState(_) => CuaActionKind::VerifyState,
            Self::StartSession(_) => CuaActionKind::StartSession,
            Self::GetSession(_) => CuaActionKind::GetSession,
            Self::ListSessions(_) => CuaActionKind::ListSessions,
            Self::EndSession(_) => CuaActionKind::EndSession,
            Self::HealthReport(_) => CuaActionKind::HealthReport,
        }
    }
    pub const fn required_capability(&self) -> Capability {
        match self {
            Self::ListApps(_) => Capability::AppDiscovery,
            Self::LaunchApp(_) => Capability::AppLaunch,
            Self::ListWindows(_) => Capability::WindowDiscovery,
            Self::GetWindowState(_) => Capability::WindowObservation,
            Self::SetWindowFrame(_) => Capability::WindowManagement,
            Self::Click(_)
            | Self::DoubleClick(_)
            | Self::RightClick(_)
            | Self::MoveCursor(_)
            | Self::Drag(_)
            | Self::Scroll(_) => Capability::Pointer,
            Self::TypeText(_) | Self::PressKey(_) | Self::Hotkey(_) => Capability::Keyboard,
            Self::SetValue(_) => Capability::ElementValue,
            Self::InvokeMenu(_) => Capability::Menu,
            Self::VerifyState(_) => Capability::Verification,
            Self::StartSession(_)
            | Self::GetSession(_)
            | Self::ListSessions(_)
            | Self::EndSession(_) => Capability::SessionLifecycle,
            Self::HealthReport(_) => Capability::Health,
        }
    }
    pub fn required_permissions(&self) -> Vec<PermissionKind> {
        use PermissionKind::*;
        match self {
            Self::GetWindowState(args) => {
                let mut required = Vec::new();
                if args.include_accessibility_tree {
                    required.push(Accessibility);
                }
                if args.include_screenshot {
                    required.push(ScreenCapture);
                }
                required
            }
            Self::VerifyState(args) => {
                let mut required = vec![Accessibility];
                if args.include_screenshot {
                    required.push(ScreenCapture);
                }
                required
            }
            Self::SetWindowFrame(_)
            | Self::Click(_)
            | Self::DoubleClick(_)
            | Self::RightClick(_)
            | Self::MoveCursor(_)
            | Self::Drag(_)
            | Self::Scroll(_)
            | Self::TypeText(_)
            | Self::PressKey(_)
            | Self::Hotkey(_)
            | Self::SetValue(_)
            | Self::InvokeMenu(_) => vec![Accessibility],
            _ => vec![],
        }
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::LaunchApp(args) => {
                if args.bundle_id.is_some() == args.name.is_some() {
                    return Err(ValidationError::new("exactly one app selector is required"));
                }
            }
            Self::GetWindowState(args) => {
                args.target.validate()?;
                if !args.include_accessibility_tree && !args.include_screenshot {
                    return Err(ValidationError::new(
                        "window state must include at least one modality",
                    ));
                }
                if args
                    .max_elements
                    .is_some_and(|v| v == 0 || usize::from(v) > MAX_ELEMENTS)
                    || args.max_depth.is_some_and(|v| v == 0 || v > 64)
                    || args.max_dimension.is_some_and(|v| v == 0 || v > 8_192)
                {
                    return Err(ValidationError::new("window state option out of range"));
                }
            }
            Self::SetWindowFrame(args) => {
                args.target.validate()?;
                args.frame.validate()?;
            }
            Self::Click(args) => {
                validate_addressed(args.target, &args.address)?;
                if !args.modifiers.is_empty() {
                    return Err(ValidationError::new(
                        "modified pointer actions require forbidden foreground delivery",
                    ));
                }
                match &args.address {
                    ElementAddress::Point(_) => {
                        if args.action != ClickAction::Press {
                            return Err(ValidationError::new(
                                "AX-only click action cannot target a point",
                            ));
                        }
                        if args.count.is_some_and(|count| count == 0 || count > 3) {
                            return Err(ValidationError::new("click count out of range"));
                        }
                    }
                    ElementAddress::ElementToken { .. } | ElementAddress::ElementIndex { .. } => {
                        if args.count.is_some() {
                            return Err(ValidationError::new(
                                "click count is only valid for pixel addressing",
                            ));
                        }
                        if args.button == MouseButton::Middle {
                            return Err(ValidationError::new(
                                "middle button has no background accessibility route",
                            ));
                        }
                    }
                }
            }
            Self::DoubleClick(args) => validate_addressed(args.target, &args.address)?,
            Self::RightClick(args) => {
                validate_addressed(args.target, &args.address)?;
                if !args.modifiers.is_empty() {
                    return Err(ValidationError::new(
                        "modified pointer actions require forbidden foreground delivery",
                    ));
                }
            }
            Self::MoveCursor(args) => {
                args.target.validate()?;
                args.point.validate()?;
            }
            Self::Drag(args) => {
                args.target.validate()?;
                args.from.validate()?;
                args.to.validate()?;
                if !args.modifiers.is_empty() {
                    return Err(ValidationError::new(
                        "modified pointer actions require forbidden foreground delivery",
                    ));
                }
                if args.duration_ms > 10_000 || args.steps == 0 || args.steps > 200 {
                    return Err(ValidationError::new("drag option out of range"));
                }
            }
            Self::Scroll(args) => {
                validate_addressed(args.target, &args.address)?;
                if args.amount == 0 || args.amount > 50 {
                    return Err(ValidationError::new("scroll amount out of range"));
                }
            }
            Self::TypeText(args) => {
                validate_addressed(args.target, &args.address)?;
                if args.delay_ms > 200 {
                    return Err(ValidationError::new("type delay out of range"));
                }
            }
            Self::PressKey(args) => {
                validate_addressed(args.target, &args.address)?;
                validate_modifiers(&args.modifiers)?;
            }
            Self::Hotkey(args) => {
                validate_addressed(args.target, &args.address)?;
                if args.keys.len() < 2 || args.keys.len() > 6 {
                    return Err(ValidationError::new("hotkey size out of range"));
                }
                let last = args.keys.len() - 1;
                if args.keys[..last]
                    .iter()
                    .any(|key| !is_hotkey_modifier(key.as_str()))
                    || is_hotkey_modifier(args.keys[last].as_str())
                {
                    return Err(ValidationError::new(
                        "hotkey requires modifiers first and one non-modifier last",
                    ));
                }
                let mut modifiers = HashSet::new();
                if args.keys[..last]
                    .iter()
                    .any(|key| !modifiers.insert(key.as_str()))
                {
                    return Err(ValidationError::new("duplicate hotkey modifier"));
                }
            }
            Self::SetValue(args) => args.target.validate()?,
            Self::InvokeMenu(args) => {
                args.target.validate()?;
                if args.path.is_empty() || args.path.len() > MAX_MENU_DEPTH {
                    return Err(ValidationError::new("menu path size out of range"));
                }
                if args
                    .path
                    .iter()
                    .any(|part| part.as_str().len() > 200 || part.as_str().trim() != part.as_str())
                {
                    return Err(ValidationError::new("invalid menu path component"));
                }
            }
            Self::VerifyState(args) => validate_verify(args)?,
            Self::ListSessions(args) => {
                if args
                    .limit
                    .is_some_and(|v| v == 0 || usize::from(v) > MAX_SESSIONS)
                {
                    return Err(ValidationError::new("session page limit out of range"));
                }
            }
            Self::HealthReport(args) => {
                if args.include.len() > 8 || args.skip.len() > 8 {
                    return Err(ValidationError::new("too many health checks"));
                }
                reject_duplicates(&args.include, "health include")?;
                reject_duplicates(&args.skip, "health skip")?;
            }
            _ => {}
        }
        Ok(())
    }
}

fn validate_addressed(
    target: WindowTarget,
    address: &ElementAddress,
) -> Result<(), ValidationError> {
    target.validate()?;
    address.validate()
}
fn validate_modifiers(values: &[Modifier]) -> Result<(), ValidationError> {
    if values.len() > MAX_MODIFIERS {
        return Err(ValidationError::new("too many modifiers"));
    }
    reject_duplicates(values, "modifier")
}
fn reject_duplicates<T: Eq + std::hash::Hash>(
    values: &[T],
    name: &str,
) -> Result<(), ValidationError> {
    let mut seen = HashSet::new();
    if values.iter().any(|value| !seen.insert(value)) {
        Err(ValidationError::new(format!("duplicate {name}")))
    } else {
        Ok(())
    }
}
fn validate_verify(args: &VerifyStateArgs) -> Result<(), ValidationError> {
    args.target.validate()?;
    if args.expect.is_empty() || args.expect.len() > MAX_VERIFY_PREDICATES {
        return Err(ValidationError::new("verify predicate count out of range"));
    }
    if args.stable_samples == 0 || args.stable_samples > 5 || args.timeout_ms > 10_000 {
        return Err(ValidationError::new("verify timing out of range"));
    }
    for predicate in &args.expect {
        match predicate {
            VerifyPredicate::WindowBounds {
                bounds,
                tolerance_px,
            } => {
                bounds.validate()?;
                finite(*tolerance_px, "tolerance_px")?;
                if !(0.0..=100.0).contains(tolerance_px) {
                    return Err(ValidationError::new("tolerance out of range"));
                }
            }
            VerifyPredicate::Element(element) => {
                if element.selector.role.is_none() && element.selector.label_contains.is_none() {
                    return Err(ValidationError::new("element selector cannot be empty"));
                }
            }
            VerifyPredicate::WindowExists(_) => {}
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CuaRequestEnvelope {
    pub version: ProtocolVersion,
    pub request_id: RequestId,
    pub machine_id: MachineId,
    pub action: CuaAction,
}
impl CuaRequestEnvelope {
    pub fn from_json(json: &str) -> Result<Self, ValidationError> {
        decode_limited(json, MAX_REQUEST_BYTES, "request", |value: Self| {
            value.validate()?;
            Ok(value)
        })
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.action.validate()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionEffect {
    Confirmed,
    Partial,
    Unverifiable,
    SuspectedNoop,
    Refused,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionRoute {
    Accessibility,
    SyntheticEvents,
    GlobalInput,
    Dom,
    TrustedInput,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionDelivery {
    pub requested: DeliveryMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_count: Option<u16>,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionEvidence {
    AccessibilityReadback,
    WindowReadback,
    Snapshot,
    Screenshot,
    DeliveryReceipt,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EscalationTarget {
    Pixel,
    Foreground,
    Page,
    Session,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EscalationReason {
    RouteUnavailable,
    DeliveryFailed,
    EffectUnconfirmed,
    SuspectedNoop,
    PermissionRequired,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionEscalation {
    pub target: EscalationTarget,
    pub reason: EscalationReason,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActionOutcome {
    pub effect: ActionEffect,
    pub route: ActionRoute,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<ActionDelivery>,
    pub evidence: Vec<ActionEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation: Option<ActionEscalation>,
}
impl ActionOutcome {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.evidence.len() > 8 {
            return Err(ValidationError::new("too much action evidence"));
        }
        reject_duplicates(&self.evidence, "action evidence")?;
        let delivery = self
            .delivery
            .as_ref()
            .ok_or_else(|| ValidationError::new("action outcome requires requested delivery"))?;
        if delivery.requested != DeliveryMode::Background {
            return Err(ValidationError::new("only background delivery is allowed"));
        }
        if delivery
            .delivered_count
            .is_some_and(|count| count == 0 || count > 10_000)
        {
            return Err(ValidationError::new("delivered_count out of range"));
        }
        if self.route == ActionRoute::Dom {
            return Err(ValidationError::new(
                "DOM route is outside the native protocol",
            ));
        }
        if matches!(
            self.route,
            ActionRoute::GlobalInput | ActionRoute::TrustedInput
        ) && !matches!(
            self.effect,
            ActionEffect::Refused | ActionEffect::Unverifiable
        ) {
            return Err(ValidationError::new(
                "global routes cannot be successful under background-only policy",
            ));
        }
        if self.effect == ActionEffect::Confirmed
            && self.escalation.as_ref().is_some_and(|escalation| {
                matches!(
                    escalation.reason,
                    EscalationReason::EffectUnconfirmed | EscalationReason::SuspectedNoop
                )
            })
        {
            return Err(ValidationError::new(
                "confirmed outcome contradicts escalation reason",
            ));
        }
        if self.effect == ActionEffect::Refused
            && self.evidence.is_empty()
            && self.escalation.is_none()
        {
            return Err(ValidationError::new(
                "refused outcome requires evidence or escalation",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClickActionResult {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub address: ElementAddress,
    pub button: MouseButton,
    pub action: ClickAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u8>,
    pub outcome: ActionOutcome,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AddressedActionResult {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub address: ElementAddress,
    pub outcome: ActionOutcome,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MoveCursorActionResult {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub point: WindowPoint,
    pub outcome: ActionOutcome,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DragActionResult {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub from: WindowPoint,
    pub to: WindowPoint,
    pub duration_ms: u16,
    pub steps: u8,
    pub button: MouseButton,
    pub outcome: ActionOutcome,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScrollActionResult {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub address: ElementAddress,
    pub direction: ScrollDirection,
    pub by: ScrollGranularity,
    pub amount: u8,
    pub outcome: ActionOutcome,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TypeTextActionResult {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub address: ElementAddress,
    pub text: BoundedText,
    pub delay_ms: u8,
    pub outcome: ActionOutcome,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PressKeyActionResult {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub address: ElementAddress,
    pub key: KeyName,
    pub modifiers: Vec<Modifier>,
    pub outcome: ActionOutcome,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HotkeyActionResult {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub address: ElementAddress,
    pub keys: Vec<HotkeyKey>,
    pub outcome: ActionOutcome,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetValueActionResult {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub element: ElementRef,
    pub value: EmptyValueText,
    pub outcome: ActionOutcome,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvokeMenuActionResult {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub path: Vec<BoundedText>,
    pub outcome: ActionOutcome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppKind {
    Desktop,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppRecord {
    pub pid: u32,
    pub bundle_id: AppBundleId,
    pub name: AppDisplayName,
    pub kind: AppKind,
    pub running: bool,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_path: Option<OutputPath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<Timestamp>,
    pub windows: Vec<WindowRecord>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppsResult {
    pub apps: Vec<AppRecord>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowRecord {
    pub target: WindowTarget,
    pub app_name: AppDisplayName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<EmptyTitleText>,
    pub bounds: Rect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z_index: Option<i32>,
    pub is_on_screen: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_ids: Option<Vec<u64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_space_id: Option<u64>,
    pub layer: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_current_space: Option<bool>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_space_id: Option<u64>,
    pub windows: Vec<WindowRecord>,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchState {
    RequestSent,
    ProcessRunning,
    WindowReady,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchAppResult {
    pub pid: u32,
    pub bundle_id: AppBundleId,
    pub name: AppDisplayName,
    pub launch_state: LaunchState,
    pub windows: Vec<WindowRecord>,
    pub self_activation_suppressed: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AccessibilityElement {
    pub element_index: u32,
    pub element_token: ElementToken,
    pub role: BoundedText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<BoundedText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<EmptyValueText>,
    pub actions: Vec<BoundedText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<Rect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_index: Option<u32>,
    pub depth: u8,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageMediaType {
    Png,
    Jpeg,
}
string_type!(Base64Image, MAX_RESULT_BYTES, validate_base64);
fn validate_base64(name: &str, value: &str, max: usize) -> Result<(), ValidationError> {
    let padding = value.bytes().rev().take_while(|byte| *byte == b'=').count();
    let valid = !value.is_empty()
        && value.len() <= max
        && value.len().is_multiple_of(4)
        && padding <= 2
        && value[..value.len() - padding]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/'));
    if valid {
        Ok(())
    } else {
        Err(ValidationError::new(format!("invalid {name}")))
    }
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Screenshot {
    pub media_type: ImageMediaType,
    pub base64: Base64Image,
    pub width: u32,
    pub height: u32,
}
impl Screenshot {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.width == 0 || self.height == 0 || self.width > 32_768 || self.height > 32_768 {
            return Err(ValidationError::new("invalid screenshot dimensions"));
        }
        let decoded = BASE64_STANDARD
            .decode(self.base64.as_str())
            .map_err(|_| ValidationError::new("invalid screenshot base64"))?;
        if decoded.is_empty() || decoded.len() > MAX_DECODED_IMAGE_BYTES {
            return Err(ValidationError::new("invalid screenshot decoded size"));
        }
        let dimensions = match self.media_type {
            ImageMediaType::Png => png_dimensions(&decoded),
            ImageMediaType::Jpeg => jpeg_dimensions(&decoded),
        }?;
        if dimensions != (self.width, self.height) {
            return Err(ValidationError::new(
                "screenshot dimensions contradict encoded image header",
            ));
        }
        Ok(())
    }
}

fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32), ValidationError> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 33
        || &bytes[..8] != SIGNATURE
        || &bytes[12..16] != b"IHDR"
        || u32::from_be_bytes(bytes[8..12].try_into().unwrap()) != 13
    {
        return Err(ValidationError::new("invalid or truncated PNG"));
    }
    let dimensions = (
        u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
        u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
    );
    let mut offset = 8usize;
    let mut saw_data = false;
    let mut saw_end = false;
    while offset + 12 <= bytes.len() {
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let end = offset
            .checked_add(12)
            .and_then(|value| value.checked_add(length))
            .ok_or_else(|| ValidationError::new("PNG chunk length overflow"))?;
        if end > bytes.len() {
            return Err(ValidationError::new("truncated PNG chunk"));
        }
        let kind = &bytes[offset + 4..offset + 8];
        saw_data |= kind == b"IDAT";
        if kind == b"IEND" {
            if length != 0 || end != bytes.len() {
                return Err(ValidationError::new("invalid PNG end chunk"));
            }
            saw_end = true;
            break;
        }
        offset = end;
    }
    if !saw_data || !saw_end || dimensions.0 == 0 || dimensions.1 == 0 {
        return Err(ValidationError::new("incomplete PNG"));
    }
    Ok(dimensions)
}

fn jpeg_dimensions(bytes: &[u8]) -> Result<(u32, u32), ValidationError> {
    if bytes.len() < 4 || bytes[..2] != [0xff, 0xd8] || bytes[bytes.len() - 2..] != [0xff, 0xd9] {
        return Err(ValidationError::new("invalid or truncated JPEG"));
    }
    let mut offset = 2usize;
    while offset < bytes.len() {
        while offset < bytes.len() && bytes[offset] == 0xff {
            offset += 1;
        }
        if offset >= bytes.len() {
            break;
        }
        let marker = bytes[offset];
        offset += 1;
        if matches!(marker, 0xd8 | 0xd9) || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        if offset + 2 > bytes.len() {
            break;
        }
        let length = usize::from(u16::from_be_bytes([bytes[offset], bytes[offset + 1]]));
        if length < 2 || offset + length > bytes.len() {
            return Err(ValidationError::new("invalid or truncated JPEG segment"));
        }
        let start_of_frame = matches!(
            marker,
            0xc0 | 0xc1
                | 0xc2
                | 0xc3
                | 0xc5
                | 0xc6
                | 0xc7
                | 0xc9
                | 0xca
                | 0xcb
                | 0xcd
                | 0xce
                | 0xcf
        );
        if start_of_frame {
            if length < 7 {
                return Err(ValidationError::new("invalid JPEG frame header"));
            }
            let height = u32::from(u16::from_be_bytes([bytes[offset + 3], bytes[offset + 4]]));
            let width = u32::from(u16::from_be_bytes([bytes[offset + 5], bytes[offset + 6]]));
            if width == 0 || height == 0 {
                return Err(ValidationError::new("invalid JPEG dimensions"));
            }
            return Ok((width, height));
        }
        if marker == 0xda {
            break;
        }
        offset += length;
    }
    Err(ValidationError::new("JPEG has no bounded frame header"))
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExactWindowStatus {
    Available,
    AxUnresolved,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationStatus {
    Fresh,
    Stale,
    Unknown,
    Unavailable,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundRouteKind {
    Accessibility,
    WindowPointer,
    PidKeyboard,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundRouteStatus {
    Available,
    Refused,
    Unverifiable,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExactWindowObservation {
    pub pid: u32,
    pub window_id: u64,
    pub status: ExactWindowStatus,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationAvailability {
    pub frame_freshness: ObservationStatus,
    pub one_shot_capture: ObservationStatus,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackgroundRoute {
    pub reason: BoundedText,
    pub route: BackgroundRouteKind,
    pub status: BackgroundRouteStatus,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackgroundInputObservation {
    pub exact_window: ExactWindowObservation,
    pub observation: ObservationAvailability,
    pub routes: Vec<BackgroundRoute>,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationEscalationTarget {
    Foreground,
    Resnapshot,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationEscalation {
    pub reason: BoundedText,
    pub recommended: ObservationEscalationTarget,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowStateResult {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<SnapshotId>,
    pub elements: Vec<AccessibilityElement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_markdown: Option<TreeMarkdown>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<Screenshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_bounds: Option<Rect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_scale: Option<f64>,
    pub truncated: bool,
    #[serde(default)]
    pub degraded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded_reason: Option<BoundedText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_name: Option<AppDisplayName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_title: Option<EmptyTitleText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elements_complete: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returned_element_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_element_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_input: Option<BackgroundInputObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation: Option<ObservationEscalation>,
}
impl WindowStateResult {
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.target.validate()?;
        if let Some(window_bounds) = &self.window_bounds {
            window_bounds.validate()?;
        }
        if self.elements.len() > MAX_ELEMENTS {
            return Err(ValidationError::new("too many accessibility elements"));
        }
        if !self.elements.is_empty() && self.snapshot_id.is_none() {
            return Err(ValidationError::new("elements require snapshot_id"));
        }
        if self
            .screenshot_scale
            .is_some_and(|s| !s.is_finite() || s <= 0.0)
        {
            return Err(ValidationError::new("invalid screenshot scale"));
        }
        if self.screenshot_scale.is_some() && self.screenshot.is_none() {
            return Err(ValidationError::new(
                "screenshot scale requires a screenshot",
            ));
        }
        if let Some(screenshot) = &self.screenshot {
            screenshot.validate()?;
        }
        if self.degraded != self.degraded_reason.is_some() {
            return Err(ValidationError::new(
                "degraded status and degraded_reason must agree",
            ));
        }
        if self
            .returned_element_count
            .is_some_and(|count| count as usize != self.elements.len())
        {
            return Err(ValidationError::new("returned element count mismatch"));
        }
        if let (Some(returned), Some(total)) =
            (self.returned_element_count, self.total_element_count)
            && returned > total
        {
            return Err(ValidationError::new("invalid element counts"));
        }
        if let Some(background) = &self.background_input {
            if background.routes.len() > 8 {
                return Err(ValidationError::new("too many background routes"));
            }
            if background.exact_window.pid != self.target.pid
                || background.exact_window.window_id != self.target.window_id
            {
                return Err(ValidationError::new(
                    "background observation target mismatch",
                ));
            }
        }
        let mut indexes = HashSet::new();
        let mut tokens = HashSet::new();
        let mut depths = std::collections::HashMap::new();
        for element in &self.elements {
            if !indexes.insert(element.element_index) {
                return Err(ValidationError::new(
                    "duplicate accessibility element index",
                ));
            }
            if !tokens.insert(&element.element_token) {
                return Err(ValidationError::new(
                    "duplicate accessibility element token",
                ));
            }
            if let Some(frame) = element.frame {
                frame.validate()?;
            }
            match element.parent_index {
                None if element.depth != 0 => {
                    return Err(ValidationError::new("non-root element requires parent"));
                }
                Some(parent) => {
                    let parent_depth = depths
                        .get(&parent)
                        .ok_or_else(|| ValidationError::new("element parent must precede child"))?;
                    if element.depth != parent_depth + 1 {
                        return Err(ValidationError::new("invalid accessibility element depth"));
                    }
                }
                None => {}
            }
            depths.insert(element.element_index, element.depth);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetWindowFrameResult {
    pub target: WindowTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub requested_frame: Rect,
    pub observed_frame: Rect,
    pub outcome: ActionOutcome,
}
impl SetWindowFrameResult {
    fn validate(&self) -> Result<(), ValidationError> {
        self.target.validate()?;
        self.requested_frame.validate()?;
        self.observed_frame.validate()?;
        self.outcome.validate()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PredicateStatus {
    Satisfied,
    Unsatisfied,
    Unknown,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PredicateEvaluation {
    pub predicate_index: u8,
    pub status: PredicateStatus,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationResult {
    pub overall: PredicateStatus,
    pub predicates: Vec<PredicateEvaluation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<Screenshot>,
}
impl VerificationResult {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.predicates.is_empty() || self.predicates.len() > MAX_VERIFY_PREDICATES {
            return Err(ValidationError::new(
                "verification result count out of range",
            ));
        }
        let mut indexes = HashSet::new();
        if self
            .predicates
            .iter()
            .any(|p| !indexes.insert(p.predicate_index))
        {
            return Err(ValidationError::new("duplicate predicate result"));
        }
        if let Some(screenshot) = &self.screenshot {
            screenshot.validate()?;
        }
        let expected = if self
            .predicates
            .iter()
            .any(|predicate| predicate.status == PredicateStatus::Unsatisfied)
        {
            PredicateStatus::Unsatisfied
        } else if self
            .predicates
            .iter()
            .any(|predicate| predicate.status == PredicateStatus::Unknown)
        {
            PredicateStatus::Unknown
        } else {
            PredicateStatus::Satisfied
        };
        if self.overall != expected {
            return Err(ValidationError::new(
                "verification overall contradicts predicates",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Active,
    Ended,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureScope {
    Auto,
    Window,
    Desktop,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StartSessionResult {
    pub active: bool,
    pub capture_scope: CaptureScope,
    pub effective_scope: CaptureScope,
    pub desktop_capture_authorized: bool,
    pub desktop_unlocked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation_detail: Option<BoundedText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation_reason: Option<BoundedText>,
    pub revived: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GetSessionResult {
    pub client_kind: BoundedText,
    pub cursor_visible: bool,
    pub expires_in_seconds: u64,
    pub idle_seconds: u64,
    pub implicit: bool,
    pub recording_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub state: SessionState,
    pub transport: BoundedText,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionListResult {
    pub sessions: Vec<GetSessionResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<PageCursor>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EndSessionResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionLabel>,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthOverall {
    Ok,
    Degraded,
    Failed,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum HealthSchemaVersion {
    #[serde(rename = "1")]
    V1,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum HealthPlatform {
    #[serde(rename = "darwin")]
    Darwin,
    #[serde(rename = "win32")]
    Win32,
    #[serde(rename = "linux")]
    Linux,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthCheckStatus {
    Pass,
    Fail,
    Skip,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformHealthData {
    pub architecture: BoundedText,
    pub os_version: BoundedText,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BundleIdentityHealthData {
    pub bundle_identifier: AppBundleId,
    pub executable_path: OutputPath,
    pub identity_source: BoundedText,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BundlePermissionHealthData {
    pub bundle_identifier: AppBundleId,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum HealthDataScalar {
    String(EmptyValueText),
    Bool(bool),
    Integer(i64),
    Number(f64),
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum HealthDataValue {
    Scalar(HealthDataScalar),
    Array(Vec<HealthDataScalar>),
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum HealthCheckData {
    Platform(PlatformHealthData),
    BundleIdentity(BundleIdentityHealthData),
    BundlePermission(BundlePermissionHealthData),
    Unknown(BTreeMap<HealthDataKey, HealthDataValue>),
}
impl HealthCheckData {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Self::Unknown(fields) = self {
            if fields.len() > MAX_HEALTH_DATA_FIELDS {
                return Err(ValidationError::new("too many health data fields"));
            }
            for value in fields.values() {
                match value {
                    HealthDataValue::Scalar(HealthDataScalar::Number(number))
                        if !number.is_finite() =>
                    {
                        return Err(ValidationError::new("non-finite health data number"));
                    }
                    HealthDataValue::Array(items) => {
                        if items.len() > MAX_HEALTH_DATA_ITEMS {
                            return Err(ValidationError::new("too many health data items"));
                        }
                        if items.iter().any(|item| {
                            matches!(item, HealthDataScalar::Number(number) if !number.is_finite())
                        }) {
                            return Err(ValidationError::new("non-finite health data number"));
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HealthCheckResult {
    pub name: HealthCheckName,
    pub status: HealthCheckStatus,
    pub message: BoundedText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<BoundedText>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<HealthCheckData>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HealthReportResult {
    pub schema_version: HealthSchemaVersion,
    pub platform: HealthPlatform,
    pub driver_version: DriverVersion,
    pub overall: HealthOverall,
    pub checks: Vec<HealthCheckResult>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    tag = "action",
    content = "result",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum CuaActionResult {
    ListApps(AppsResult),
    LaunchApp(LaunchAppResult),
    ListWindows(WindowsResult),
    GetWindowState(Box<WindowStateResult>),
    SetWindowFrame(SetWindowFrameResult),
    Click(ClickActionResult),
    DoubleClick(AddressedActionResult),
    RightClick(AddressedActionResult),
    MoveCursor(MoveCursorActionResult),
    Drag(DragActionResult),
    Scroll(ScrollActionResult),
    TypeText(TypeTextActionResult),
    PressKey(PressKeyActionResult),
    Hotkey(HotkeyActionResult),
    SetValue(SetValueActionResult),
    InvokeMenu(InvokeMenuActionResult),
    VerifyState(VerificationResult),
    StartSession(StartSessionResult),
    GetSession(GetSessionResult),
    ListSessions(SessionListResult),
    EndSession(EndSessionResult),
    HealthReport(HealthReportResult),
}
impl CuaActionResult {
    pub const fn kind(&self) -> CuaActionKind {
        match self {
            Self::ListApps(_) => CuaActionKind::ListApps,
            Self::LaunchApp(_) => CuaActionKind::LaunchApp,
            Self::ListWindows(_) => CuaActionKind::ListWindows,
            Self::GetWindowState(_) => CuaActionKind::GetWindowState,
            Self::SetWindowFrame(_) => CuaActionKind::SetWindowFrame,
            Self::Click(_) => CuaActionKind::Click,
            Self::DoubleClick(_) => CuaActionKind::DoubleClick,
            Self::RightClick(_) => CuaActionKind::RightClick,
            Self::MoveCursor(_) => CuaActionKind::MoveCursor,
            Self::Drag(_) => CuaActionKind::Drag,
            Self::Scroll(_) => CuaActionKind::Scroll,
            Self::TypeText(_) => CuaActionKind::TypeText,
            Self::PressKey(_) => CuaActionKind::PressKey,
            Self::Hotkey(_) => CuaActionKind::Hotkey,
            Self::SetValue(_) => CuaActionKind::SetValue,
            Self::InvokeMenu(_) => CuaActionKind::InvokeMenu,
            Self::VerifyState(_) => CuaActionKind::VerifyState,
            Self::StartSession(_) => CuaActionKind::StartSession,
            Self::GetSession(_) => CuaActionKind::GetSession,
            Self::ListSessions(_) => CuaActionKind::ListSessions,
            Self::EndSession(_) => CuaActionKind::EndSession,
            Self::HealthReport(_) => CuaActionKind::HealthReport,
        }
    }
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::ListApps(result) => {
                if result.apps.len() > MAX_APPS {
                    return Err(ValidationError::new("too many apps"));
                }
                let mut pids = HashSet::new();
                for app in &result.apps {
                    if (app.active && !app.running) || (app.running == (app.pid == 0)) {
                        return Err(ValidationError::new("invalid app process state"));
                    }
                    if app.pid != 0 && !pids.insert(app.pid) {
                        return Err(ValidationError::new("duplicate app pid"));
                    }
                    validate_windows(&app.windows)?;
                    if app
                        .windows
                        .iter()
                        .any(|window| window.target.pid != app.pid)
                    {
                        return Err(ValidationError::new("app window pid mismatch"));
                    }
                }
                Ok(())
            }
            Self::ListWindows(result) => validate_windows(&result.windows),
            Self::LaunchApp(result) => {
                if result.pid == 0 {
                    return Err(ValidationError::new("invalid launch pid"));
                }
                validate_windows(&result.windows)?;
                if result
                    .windows
                    .iter()
                    .any(|window| window.target.pid != result.pid)
                {
                    return Err(ValidationError::new("launch window pid mismatch"));
                }
                Ok(())
            }
            Self::GetWindowState(result) => result.validate(),
            Self::SetWindowFrame(result) => result.validate(),
            Self::Click(result) => {
                validate_addressed(result.target, &result.address)?;
                result.outcome.validate()
            }
            Self::DoubleClick(result) | Self::RightClick(result) => {
                validate_addressed(result.target, &result.address)?;
                result.outcome.validate()
            }
            Self::MoveCursor(result) => {
                result.target.validate()?;
                result.point.validate()?;
                result.outcome.validate()
            }
            Self::Drag(result) => {
                result.target.validate()?;
                result.from.validate()?;
                result.to.validate()?;
                result.outcome.validate()
            }
            Self::Scroll(result) => {
                validate_addressed(result.target, &result.address)?;
                result.outcome.validate()
            }
            Self::TypeText(result) => {
                validate_addressed(result.target, &result.address)?;
                result.outcome.validate()
            }
            Self::PressKey(result) => {
                validate_addressed(result.target, &result.address)?;
                validate_modifiers(&result.modifiers)?;
                result.outcome.validate()
            }
            Self::Hotkey(result) => {
                validate_addressed(result.target, &result.address)?;
                result.outcome.validate()
            }
            Self::SetValue(result) => {
                result.target.validate()?;
                result.outcome.validate()
            }
            Self::InvokeMenu(result) => {
                result.target.validate()?;
                result.outcome.validate()
            }
            Self::VerifyState(result) => result.validate(),
            Self::StartSession(_) | Self::GetSession(_) => Ok(()),
            Self::ListSessions(result) => validate_session_list(result),
            Self::EndSession(_) => Ok(()),
            Self::HealthReport(result) => validate_health_report(result),
        }
    }
}

fn validate_session_list(result: &SessionListResult) -> Result<(), ValidationError> {
    if result.sessions.len() > MAX_SESSIONS {
        return Err(ValidationError::new("too many sessions"));
    }
    let mut labels = HashSet::new();
    for session in &result.sessions {
        if !labels.insert(session.session.as_ref().map(SessionLabel::as_str)) {
            return Err(ValidationError::new("duplicate session label"));
        }
    }
    Ok(())
}

fn validate_health_report(result: &HealthReportResult) -> Result<(), ValidationError> {
    if result.checks.len() > 16 {
        return Err(ValidationError::new("too many health checks"));
    }
    let mut names = HashSet::new();
    let mut core_failure = false;
    let mut non_core_failure = false;
    for check in &result.checks {
        if !names.insert(&check.name) {
            return Err(ValidationError::new("duplicate health check"));
        }
        validate_single_line("health message", check.message.as_str(), 1_024)?;
        if let Some(hint) = &check.hint {
            validate_single_line("health hint", hint.as_str(), 2_048)?;
        }
        if let Some(data) = &check.data {
            data.validate()?;
            let typed_match = match check.name.as_str() {
                "platform_supported" => matches!(data, HealthCheckData::Platform(_)),
                "bundle_identity" => matches!(data, HealthCheckData::BundleIdentity(_)),
                "tcc_accessibility" | "tcc_screen_recording" => {
                    matches!(data, HealthCheckData::BundlePermission(_))
                }
                _ => true,
            };
            if !typed_match {
                return Err(ValidationError::new("health check data shape mismatch"));
            }
        }
        if check.status == HealthCheckStatus::Fail {
            if matches!(
                check.name.as_str(),
                "binary_version" | "platform_supported" | "session_active"
            ) {
                core_failure = true;
            } else {
                non_core_failure = true;
            }
        }
    }
    let expected = if core_failure {
        HealthOverall::Failed
    } else if non_core_failure {
        HealthOverall::Degraded
    } else {
        HealthOverall::Ok
    };
    if result.overall != expected {
        return Err(ValidationError::new("health overall contradicts checks"));
    }
    Ok(())
}

fn validate_windows(windows: &[WindowRecord]) -> Result<(), ValidationError> {
    if windows.len() > MAX_WINDOWS {
        return Err(ValidationError::new("too many windows"));
    }
    let mut targets = HashSet::new();
    for window in windows {
        window.target.validate()?;
        validate_window_output_bounds(&window.bounds)?;
        if window
            .space_ids
            .as_ref()
            .is_some_and(|spaces| spaces.len() > 32)
        {
            return Err(ValidationError::new("too many window space ids"));
        }
        if !targets.insert((window.target.pid, window.target.window_id)) {
            return Err(ValidationError::new("duplicate window target"));
        }
    }
    Ok(())
}

fn validate_window_output_bounds(bounds: &Rect) -> Result<(), ValidationError> {
    finite(bounds.x, "window x")?;
    finite(bounds.y, "window y")?;
    finite(bounds.width, "window width")?;
    finite(bounds.height, "window height")?;
    if bounds.width < 0.0 || bounds.height < 0.0 {
        return Err(ValidationError::new(
            "window output dimensions cannot be negative",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeErrorCode {
    InvalidRequest,
    CapabilityDenied,
    PermissionDenied,
    MachineUnavailable,
    SessionUnavailable,
    StaleSnapshot,
    TargetUnavailable,
    DriverFailure,
    RuntimeUnavailable,
    Timeout,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CuaRuntimeError {
    pub code: RuntimeErrorCode,
    pub message: BoundedText,
    pub retryable: bool,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum CuaResponse {
    Success { result: Box<CuaActionResult> },
    Error { error: CuaRuntimeError },
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CuaResponseEnvelope {
    pub version: ProtocolVersion,
    pub request_id: RequestId,
    pub machine_id: MachineId,
    pub action: CuaActionKind,
    pub response: CuaResponse,
}
impl CuaResponseEnvelope {
    pub fn from_json(json: &str) -> Result<Self, ValidationError> {
        decode_limited(json, MAX_RESULT_BYTES, "result", |value: Self| {
            value.validate()?;
            Ok(value)
        })
    }
    pub fn validate(&self) -> Result<(), ValidationError> {
        if let CuaResponse::Success { result } = &self.response {
            if result.kind() != self.action {
                return Err(ValidationError::new("response action/result mismatch"));
            }
            result.validate()?;
        }
        Ok(())
    }
    pub fn validate_response_for(
        &self,
        request: &CuaRequestEnvelope,
    ) -> Result<(), ValidationError> {
        request.validate()?;
        self.validate()?;
        if self.version != request.version
            || self.request_id != request.request_id
            || self.machine_id != request.machine_id
            || self.action != request.action.kind()
        {
            return Err(ValidationError::new("response does not match request"));
        }
        if let CuaResponse::Success { result } = &self.response {
            validate_result_for_action(result, &request.action)?;
        }
        Ok(())
    }
}

fn validate_result_for_action(
    result: &CuaActionResult,
    action: &CuaAction,
) -> Result<(), ValidationError> {
    match (result, action) {
        (CuaActionResult::ListApps(_), CuaAction::ListApps(_)) => {}
        (CuaActionResult::LaunchApp(result), CuaAction::LaunchApp(args)) => {
            if args
                .bundle_id
                .as_ref()
                .is_some_and(|bundle| bundle != &result.bundle_id)
                || args.name.as_ref().is_some_and(|name| name != &result.name)
            {
                return Err(ValidationError::new("launch result selector mismatch"));
            }
        }
        (CuaActionResult::ListWindows(result), CuaAction::ListWindows(args)) => {
            if result
                .windows
                .iter()
                .any(|window| args.pid.is_some_and(|pid| window.target.pid != pid.get()))
            {
                return Err(ValidationError::new("window result violates pid filter"));
            }
            if args.on_screen_only && result.windows.iter().any(|window| !window.is_on_screen) {
                return Err(ValidationError::new(
                    "window result violates on_screen_only filter",
                ));
            }
        }
        (CuaActionResult::GetWindowState(result), CuaAction::GetWindowState(args)) => {
            if result.target != args.target {
                return Err(ValidationError::new("window state target mismatch"));
            }
            if args
                .max_elements
                .is_some_and(|limit| result.elements.len() > usize::from(limit))
                || args.max_depth.is_some_and(|limit| {
                    result.elements.iter().any(|element| element.depth > limit)
                })
            {
                return Err(ValidationError::new(
                    "window state exceeds requested accessibility bounds",
                ));
            }
            if !args.include_accessibility_tree
                && (result.snapshot_id.is_some()
                    || !result.elements.is_empty()
                    || result.tree_markdown.is_some())
            {
                return Err(ValidationError::new(
                    "unrequested accessibility modality returned",
                ));
            }
            if args.include_screenshot {
                if result.screenshot.is_none() && !result.degraded {
                    return Err(ValidationError::new(
                        "requested screenshot missing without degradation",
                    ));
                }
                if let (Some(limit), Some(screenshot)) = (args.max_dimension, &result.screenshot)
                    && screenshot.width.max(screenshot.height) > u32::from(limit)
                {
                    return Err(ValidationError::new(
                        "screenshot exceeds requested maximum dimension",
                    ));
                }
            } else if result.screenshot.is_some() || result.screenshot_scale.is_some() {
                return Err(ValidationError::new(
                    "unrequested screenshot modality returned",
                ));
            }
        }
        (CuaActionResult::SetWindowFrame(result), CuaAction::SetWindowFrame(args)) => {
            if result.target != args.target
                || result.session != args.session
                || result.requested_frame != args.frame
            {
                return Err(ValidationError::new("set_window_frame result mismatch"));
            }
        }
        (CuaActionResult::Click(result), CuaAction::Click(args)) => {
            if result.target != args.target
                || result.session != args.session
                || result.address != args.address
                || result.button != args.button
                || result.action != args.action
                || result.count != args.count
            {
                return Err(ValidationError::new("click result context mismatch"));
            }
        }
        (CuaActionResult::DoubleClick(result), CuaAction::DoubleClick(args)) => {
            validate_addressed_result(result, args.target, &args.session, &args.address)?;
        }
        (CuaActionResult::RightClick(result), CuaAction::RightClick(args)) => {
            validate_addressed_result(result, args.target, &args.session, &args.address)?;
        }
        (CuaActionResult::MoveCursor(result), CuaAction::MoveCursor(args)) => {
            if result.target != args.target
                || result.session != args.session
                || result.point != args.point
            {
                return Err(ValidationError::new("move_cursor result context mismatch"));
            }
        }
        (CuaActionResult::Drag(result), CuaAction::Drag(args)) => {
            if result.target != args.target
                || result.session != args.session
                || result.from != args.from
                || result.to != args.to
                || result.duration_ms != args.duration_ms
                || result.steps != args.steps
                || result.button != args.button
            {
                return Err(ValidationError::new("drag result context mismatch"));
            }
        }
        (CuaActionResult::Scroll(result), CuaAction::Scroll(args)) => {
            if result.target != args.target
                || result.session != args.session
                || result.address != args.address
                || result.direction != args.direction
                || result.by != args.by
                || result.amount != args.amount
            {
                return Err(ValidationError::new("scroll result context mismatch"));
            }
        }
        (CuaActionResult::TypeText(result), CuaAction::TypeText(args)) => {
            if result.target != args.target
                || result.session != args.session
                || result.address != args.address
                || result.text != args.text
                || result.delay_ms != args.delay_ms
            {
                return Err(ValidationError::new("type_text result context mismatch"));
            }
        }
        (CuaActionResult::PressKey(result), CuaAction::PressKey(args)) => {
            if result.target != args.target
                || result.session != args.session
                || result.address != args.address
                || result.key != args.key
                || result.modifiers != args.modifiers
            {
                return Err(ValidationError::new("press_key result context mismatch"));
            }
        }
        (CuaActionResult::Hotkey(result), CuaAction::Hotkey(args)) => {
            if result.target != args.target
                || result.session != args.session
                || result.address != args.address
                || result.keys != args.keys
            {
                return Err(ValidationError::new("hotkey result context mismatch"));
            }
        }
        (CuaActionResult::SetValue(result), CuaAction::SetValue(args)) => {
            if result.target != args.target
                || result.session != args.session
                || result.element != args.element
                || result.value != args.value
            {
                return Err(ValidationError::new("set_value result context mismatch"));
            }
        }
        (CuaActionResult::InvokeMenu(result), CuaAction::InvokeMenu(args)) => {
            if result.target != args.target
                || result.session != args.session
                || result.path != args.path
            {
                return Err(ValidationError::new("invoke_menu result context mismatch"));
            }
        }
        (CuaActionResult::VerifyState(result), CuaAction::VerifyState(args)) => {
            if result.predicates.len() != args.expect.len()
                || result
                    .predicates
                    .iter()
                    .enumerate()
                    .any(|(index, predicate)| usize::from(predicate.predicate_index) != index)
            {
                return Err(ValidationError::new(
                    "verification predicates do not correspond to request",
                ));
            }
            if result.screenshot.is_some() != args.include_screenshot {
                return Err(ValidationError::new(
                    "verification screenshot does not match request",
                ));
            }
        }
        (CuaActionResult::StartSession(result), CuaAction::StartSession(args)) => {
            if result.session != args.session || !result.active {
                return Err(ValidationError::new("started session label mismatch"));
            }
        }
        (CuaActionResult::GetSession(result), CuaAction::GetSession(args)) => {
            if result.session != args.session {
                return Err(ValidationError::new("session label mismatch"));
            }
        }
        (CuaActionResult::ListSessions(result), CuaAction::ListSessions(args)) => {
            if args
                .limit
                .is_some_and(|limit| result.sessions.len() > usize::from(limit))
            {
                return Err(ValidationError::new(
                    "session result exceeds requested limit",
                ));
            }
        }
        (CuaActionResult::EndSession(result), CuaAction::EndSession(args)) => {
            if result.session != args.session || result.active {
                return Err(ValidationError::new("ended session label mismatch"));
            }
        }
        (CuaActionResult::HealthReport(result), CuaAction::HealthReport(args)) => {
            let canonical_checks: &[&str] = match result.platform {
                HealthPlatform::Darwin => &[
                    "binary_version",
                    "platform_supported",
                    "session_active",
                    "bundle_identity",
                    "tcc_accessibility",
                    "tcc_screen_recording",
                    "ax_capability",
                    "screen_capture_capability",
                ],
                HealthPlatform::Win32 | HealthPlatform::Linux => &[
                    "binary_version",
                    "platform_supported",
                    "session_active",
                    "ax_capability",
                    "screen_capture_capability",
                ],
            };
            if (!args.include.is_empty() || !args.skip.is_empty())
                && canonical_checks.iter().any(|expected| {
                    !result
                        .checks
                        .iter()
                        .any(|check| check.name.as_str() == *expected)
                })
            {
                return Err(ValidationError::new(
                    "filtered health report omitted a canonical check",
                ));
            }
            if !args.include.is_empty()
                && result.checks.iter().any(|check| {
                    !args.include.contains(&check.name) && check.status != HealthCheckStatus::Skip
                })
            {
                return Err(ValidationError::new(
                    "unrequested health check was not marked skipped",
                ));
            }
            if args.include.is_empty()
                && result.checks.iter().any(|check| {
                    args.skip.contains(&check.name) && check.status != HealthCheckStatus::Skip
                })
            {
                return Err(ValidationError::new(
                    "filtered health check was not marked skipped",
                ));
            }
        }
        _ => return Err(ValidationError::new("unhandled action/result pairing")),
    }
    Ok(())
}

fn validate_addressed_result(
    result: &AddressedActionResult,
    target: WindowTarget,
    session: &Option<SessionLabel>,
    address: &ElementAddress,
) -> Result<(), ValidationError> {
    if result.target != target || &result.session != session || &result.address != address {
        Err(ValidationError::new(
            "addressed action result context mismatch",
        ))
    } else {
        Ok(())
    }
}

type RawAdapterFuture = Pin<Box<dyn Future<Output = CuaResponseEnvelope> + Send + 'static>>;
type RawAdapter = dyn Fn(CuaRequestEnvelope) -> RawAdapterFuture + Send + Sync;

/// The only public execution boundary for a raw Cua callback.
///
/// Its fields are private so callers cannot construct an unchecked adapter, and the raw callback
/// is never exposed. Every execution validates authorization and response correlation.
///
/// ```compile_fail
/// use cua_protocol::{CheckedCuaAdapter, MachineDescriptor};
/// fn bypass(descriptor: MachineDescriptor) {
///     let _ = CheckedCuaAdapter { descriptor, raw: unreachable!() };
/// }
/// ```
pub struct CheckedCuaAdapter {
    descriptor: MachineDescriptor,
    raw: Arc<RawAdapter>,
}
impl CheckedCuaAdapter {
    pub fn new<F>(descriptor: MachineDescriptor, raw: F) -> Result<Self, ValidationError>
    where
        F: Fn(CuaRequestEnvelope) -> RawAdapterFuture + Send + Sync + 'static,
    {
        descriptor.validate()?;
        Ok(Self {
            descriptor,
            raw: Arc::new(raw),
        })
    }

    pub fn descriptor(&self) -> &MachineDescriptor {
        &self.descriptor
    }

    pub async fn execute(
        &self,
        request: &CuaRequestEnvelope,
    ) -> Result<CuaResponseEnvelope, ValidationError> {
        request.validate()?;
        if request.machine_id != self.descriptor.machine_id {
            return Err(ValidationError::new(
                "request machine does not match adapter",
            ));
        }
        self.descriptor.authorize(&request.action)?;
        let response = (self.raw)(request.clone()).await;
        response.validate_response_for(request)?;
        Ok(response)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionError {
    NoMachines,
    Ambiguous,
    NotFound,
    DuplicateMachineId,
}
pub fn select_machine<'a>(
    machines: &'a [MachineDescriptor],
    requested: Option<&MachineId>,
) -> Result<&'a MachineDescriptor, SelectionError> {
    let mut ids = HashSet::new();
    if machines
        .iter()
        .any(|machine| !ids.insert(&machine.machine_id))
    {
        return Err(SelectionError::DuplicateMachineId);
    }
    if let Some(requested) = requested {
        return machines
            .iter()
            .find(|machine| &machine.machine_id == requested)
            .ok_or(SelectionError::NotFound);
    }
    match machines {
        [] => Err(SelectionError::NoMachines),
        [machine] => Ok(machine),
        _ => Err(SelectionError::Ambiguous),
    }
}

fn decode_limited<T, F>(
    json: &str,
    byte_limit: usize,
    name: &str,
    validate: F,
) -> Result<T, ValidationError>
where
    T: for<'de> Deserialize<'de>,
    F: FnOnce(T) -> Result<T, ValidationError>,
{
    if json.len() > byte_limit {
        return Err(ValidationError::new(format!(
            "{name} payload size limit exceeded"
        )));
    }
    validate_json_depth(json.as_bytes())?;
    let value = serde_json::from_str(json)
        .map_err(|error| ValidationError::new(format!("invalid {name}: {error}")))?;
    validate(value)
}

fn validate_json_depth(bytes: &[u8]) -> Result<(), ValidationError> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for &byte in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_JSON_DEPTH {
                    return Err(ValidationError::new("JSON depth limit exceeded"));
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}
