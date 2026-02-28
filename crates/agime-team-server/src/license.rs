//! License Key verification for brand customization.
//!
//! Uses Ed25519 asymmetric signatures with machine fingerprint binding.
//! Only the public key is compiled into the binary.
//! The private key is kept externally and used only for license generation.
//!
//! License format: `base64url(payload_json).base64url(ed25519_signature)`
//!
//! Flow:
//! 1. Customer deploys server → web UI shows machine_id
//! 2. Customer sends machine_id to vendor
//! 3. Vendor generates license key bound to that machine_id
//! 4. Customer enters key in web UI → instant activation

use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

/// Brand configuration exposed to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrandConfig {
    pub name: String,
    pub logo_text: String,
    pub logo_url: Option<String>,
    pub website_url: Option<String>,
    pub website_label: Option<String>,
    pub powered_by_visible: bool,
    pub licensed: bool,
    pub licensee: Option<String>,
    pub machine_id: String,
}

/// Internal license payload (JSON inside the key).
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct LicensePayload {
    pub iss: Option<String>,
    pub sub: String,
    pub iat: Option<u64>,
    pub machine_id: String,
    #[serde(default)]
    pub features: Vec<String>,
    pub brand: Option<LicenseBrand>,
}

/// Brand overrides embedded in the license.
#[derive(Debug, Clone, Deserialize)]
pub struct LicenseBrand {
    pub name: Option<String>,
    pub logo_text: Option<String>,
    pub logo_url: Option<String>,
    pub website_url: Option<String>,
    pub website_label: Option<String>,
}

/// Result of license verification.
#[derive(Debug, Clone)]
pub struct LicenseClaims {
    pub valid: bool,
    pub payload: Option<LicensePayload>,
    pub error_message: Option<String>,
}

// ---------------------------------------------------------------------------
// Machine fingerprint
// ---------------------------------------------------------------------------

/// Compute a stable machine fingerprint.
/// Uses OS-level machine ID when available, falls back to hostname.
/// Returns a 16-char hex string (first 8 bytes of SHA-256).
pub fn compute_machine_id() -> String {
    let raw = read_raw_machine_id();
    let hash = Sha256::digest(raw.as_bytes());
    hash.iter().take(8).map(|b| format!("{:02x}", b)).collect()
}

fn read_raw_machine_id() -> String {
    // Windows: HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid
    #[cfg(target_os = "windows")]
    {
        if let Ok(guid) = read_windows_machine_guid() {
            return guid;
        }
    }
    // Linux: /etc/machine-id
    #[cfg(target_os = "linux")]
    {
        if let Ok(id) = std::fs::read_to_string("/etc/machine-id") {
            let id = id.trim().to_string();
            if !id.is_empty() {
                return id;
            }
        }
    }
    // macOS: IOPlatformUUID via ioreg
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("ioreg")
            .args(["-rd1", "-c", "IOPlatformExpertDevice"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if line.contains("IOPlatformUUID") {
                    if let Some(uuid) = line.split('"').nth(3) {
                        return uuid.to_string();
                    }
                }
            }
        }
    }
    // Fallback: hostname from env
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown-host".into())
}

#[cfg(target_os = "windows")]
fn read_windows_machine_guid() -> Result<String, String> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hklm
        .open_subkey("SOFTWARE\\Microsoft\\Cryptography")
        .map_err(|e| e.to_string())?;
    key.get_value("MachineGuid").map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Ed25519 public key (embedded in binary, split for obfuscation)
// ---------------------------------------------------------------------------

fn embedded_public_key() -> [u8; 32] {
    let p1: [u8; 8] = [0x9c, 0x57, 0x60, 0x9a, 0x75, 0x14, 0x90, 0xd4];
    let p2: [u8; 8] = [0xae, 0x15, 0xf0, 0xf1, 0x89, 0x3e, 0x27, 0xbc];
    let p3: [u8; 8] = [0x2d, 0x5a, 0xd4, 0x5a, 0x93, 0x0c, 0x8f, 0xc9];
    let p4: [u8; 8] = [0x7b, 0xf5, 0x71, 0xc6, 0x96, 0xf2, 0xdb, 0x74];
    let mut key = [0u8; 32];
    key[..8].copy_from_slice(&p1);
    key[8..16].copy_from_slice(&p2);
    key[16..24].copy_from_slice(&p3);
    key[24..].copy_from_slice(&p4);
    key
}

// ---------------------------------------------------------------------------
// Signature verification
// ---------------------------------------------------------------------------

/// Verify a license key string using Ed25519 signature.
/// Does NOT check machine_id — call `verify_and_bind` for full validation.
pub fn verify_license(key: &str) -> LicenseClaims {
    let key = key.trim();
    if key.is_empty() {
        return LicenseClaims {
            valid: false,
            payload: None,
            error_message: Some("Empty license key".into()),
        };
    }

    let parts: Vec<&str> = key.rsplitn(2, '.').collect();
    if parts.len() != 2 {
        return LicenseClaims {
            valid: false,
            payload: None,
            error_message: Some("Invalid license format".into()),
        };
    }
    let (sig_b64, payload_b64) = (parts[0], parts[1]);
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let payload_bytes = match engine.decode(payload_b64) {
        Ok(b) => b,
        Err(e) => return err_claims(format!("Decode payload: {}", e)),
    };
    let sig_bytes = match engine.decode(sig_b64) {
        Ok(b) => b,
        Err(e) => return err_claims(format!("Decode signature: {}", e)),
    };
    let signature = match Signature::from_slice(&sig_bytes) {
        Ok(s) => s,
        Err(_) => return err_claims("Invalid signature length".into()),
    };

    let vk = match VerifyingKey::from_bytes(&embedded_public_key()) {
        Ok(k) => k,
        Err(_) => return err_claims("Internal: invalid public key".into()),
    };
    if vk.verify(payload_b64.as_bytes(), &signature).is_err() {
        return err_claims("Invalid signature".into());
    }

    match serde_json::from_slice::<LicensePayload>(&payload_bytes) {
        Ok(p) => LicenseClaims {
            valid: true,
            payload: Some(p),
            error_message: None,
        },
        Err(e) => err_claims(format!("Parse payload: {}", e)),
    }
}

fn err_claims(msg: String) -> LicenseClaims {
    LicenseClaims {
        valid: false,
        payload: None,
        error_message: Some(msg),
    }
}

// ---------------------------------------------------------------------------
// Brand config
// ---------------------------------------------------------------------------

/// Default Agime brand (no valid license).
pub fn default_brand() -> BrandConfig {
    BrandConfig {
        name: "Agime Team".into(),
        logo_text: "A".into(),
        logo_url: None,
        website_url: Some("https://www.agiatme.com".into()),
        website_label: Some("Agime Official Website".into()),
        powered_by_visible: true,
        licensed: false,
        licensee: None,
        machine_id: compute_machine_id(),
    }
}

/// Build BrandConfig from verified license claims.
pub fn brand_from_claims(claims: &LicenseClaims) -> BrandConfig {
    let payload = match (&claims.valid, &claims.payload) {
        (true, Some(p)) => p,
        _ => return default_brand(),
    };

    let has_white_label = payload.features.iter().any(|f| f == "white_label");
    let mut brand = default_brand();
    brand.licensed = true;
    brand.licensee = Some(payload.sub.clone());

    if let Some(ref b) = payload.brand {
        if let Some(ref v) = b.name {
            brand.name.clone_from(v);
        }
        if let Some(ref v) = b.logo_text {
            brand.logo_text.clone_from(v);
        }
        brand.logo_url.clone_from(&b.logo_url);
        brand.website_url.clone_from(&b.website_url);
        brand.website_label.clone_from(&b.website_label);
    }

    if has_white_label {
        brand.powered_by_visible = false;
    }
    brand
}

// ---------------------------------------------------------------------------
// License key persistence & activation
// ---------------------------------------------------------------------------

const LICENSE_FILE: &str = ".license_key";
const BRAND_OVERRIDES_FILE: &str = ".brand_overrides.json";

/// User-editable brand overrides (only effective when licensed).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrandOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logo_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website_label: Option<String>,
}

/// Read stored license key from file.
pub fn read_license_file(data_dir: &std::path::Path) -> Option<String> {
    let path = data_dir.join(LICENSE_FILE);
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Write license key to file.
fn write_license_file(data_dir: &std::path::Path, key: &str) -> Result<(), String> {
    let path = data_dir.join(LICENSE_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Create dir: {}", e))?;
    }
    std::fs::write(path, key).map_err(|e| format!("Write license file: {}", e))
}

/// Read brand overrides from file.
pub fn read_brand_overrides(data_dir: &std::path::Path) -> BrandOverrides {
    let path = data_dir.join(BRAND_OVERRIDES_FILE);
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Write brand overrides to file.
pub fn write_brand_overrides(
    data_dir: &std::path::Path,
    overrides: &BrandOverrides,
) -> Result<(), String> {
    let path = data_dir.join(BRAND_OVERRIDES_FILE);
    let json = serde_json::to_string_pretty(overrides).map_err(|e| format!("Serialize: {}", e))?;
    std::fs::write(path, json).map_err(|e| format!("Write overrides: {}", e))
}

/// Apply user overrides on top of a BrandConfig (only when licensed).
pub fn apply_overrides(brand: &mut BrandConfig, overrides: &BrandOverrides) {
    if !brand.licensed {
        return;
    }
    if let Some(ref v) = overrides.name {
        brand.name.clone_from(v);
    }
    if let Some(ref v) = overrides.logo_text {
        brand.logo_text.clone_from(v);
    }
    brand.logo_url.clone_from(&overrides.logo_url);
    brand.website_url.clone_from(&overrides.website_url);
    brand.website_label.clone_from(&overrides.website_label);
}

/// Verify signature + check machine_id match. Full validation.
pub fn verify_and_bind(key: &str) -> Result<LicenseClaims, String> {
    let claims = verify_license(key);
    if !claims.valid {
        return Err(claims.error_message.unwrap_or("Invalid license".into()));
    }
    let payload = claims.payload.as_ref().unwrap();
    let local_mid = compute_machine_id();
    if payload.machine_id != local_mid {
        return Err(format!(
            "Machine ID mismatch: license is for '{}', this machine is '{}'",
            payload.machine_id, local_mid
        ));
    }
    Ok(claims)
}

/// Activate a license key: verify, check machine binding, persist to file.
/// Returns the new BrandConfig on success.
pub fn activate_license(key: &str, data_dir: &std::path::Path) -> Result<BrandConfig, String> {
    let claims = verify_and_bind(key)?;
    write_license_file(data_dir, key)?;
    let brand = brand_from_claims(&claims);
    info!(
        "License activated: licensee={}, brand={}",
        brand.licensee.as_deref().unwrap_or("?"),
        brand.name
    );
    Ok(brand)
}

/// Resolve brand config on startup.
/// Reads license key from env var or persisted file, verifies and binds.
pub fn resolve_brand(env_key: Option<&str>, data_dir: &std::path::Path) -> BrandConfig {
    // Prefer env var, fall back to persisted file
    let key = env_key
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| read_license_file(data_dir));

    let mut brand = match key {
        None => {
            info!("No license key found, using default branding");
            default_brand()
        }
        Some(k) => match verify_and_bind(&k) {
            Ok(claims) => {
                let b = brand_from_claims(&claims);
                info!(
                    "License OK: licensee={}, brand={}",
                    b.licensee.as_deref().unwrap_or("?"),
                    b.name
                );
                b
            }
            Err(e) => {
                warn!("License invalid: {}", e);
                default_brand()
            }
        },
    };

    // Apply user-editable brand overrides (apply_overrides is a no-op when unlicensed)
    let overrides = read_brand_overrides(data_dir);
    apply_overrides(&mut brand, &overrides);

    brand
}

// ---------------------------------------------------------------------------
// Key generation (vendor-side)
// ---------------------------------------------------------------------------

/// Generate a new Ed25519 keypair. Returns (private_key_hex, public_key_hex).
pub fn generate_keypair() -> (String, String) {
    let mut csprng = rand::rngs::OsRng;
    let signing_key = SigningKey::generate(&mut csprng);
    let verifying_key = signing_key.verifying_key();
    (
        bytes_to_hex(&signing_key.to_bytes()),
        bytes_to_hex(&verifying_key.to_bytes()),
    )
}

/// Generate a signed license key bound to a specific machine_id.
pub fn generate_license_key(
    signing_key_hex: &str,
    licensee: &str,
    machine_id: &str,
    brand_name: Option<&str>,
    brand_logo_text: Option<&str>,
    brand_logo_url: Option<&str>,
    brand_website_url: Option<&str>,
    brand_website_label: Option<&str>,
    features: Vec<String>,
) -> Result<String, String> {
    let key_bytes = bytes_from_hex(signing_key_hex)?;
    if key_bytes.len() != 32 {
        return Err(format!(
            "Signing key must be 32 bytes, got {}",
            key_bytes.len()
        ));
    }
    let mut key_arr = [0u8; 32];
    key_arr.copy_from_slice(&key_bytes);
    let signing_key = SigningKey::from_bytes(&key_arr);
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("System time error: {}", e))?
        .as_secs();

    let brand = if brand_name.is_some()
        || brand_logo_text.is_some()
        || brand_logo_url.is_some()
        || brand_website_url.is_some()
        || brand_website_label.is_some()
    {
        Some(serde_json::json!({
            "name": brand_name,
            "logo_text": brand_logo_text,
            "logo_url": brand_logo_url,
            "website_url": brand_website_url,
            "website_label": brand_website_label,
        }))
    } else {
        None
    };

    let payload = serde_json::json!({
        "iss": "agime",
        "sub": licensee,
        "iat": now,
        "machine_id": machine_id,
        "features": features,
        "brand": brand,
    });

    let payload_json = serde_json::to_string(&payload).map_err(|e| format!("JSON error: {}", e))?;
    let payload_b64 = engine.encode(payload_json.as_bytes());

    let signature = signing_key.sign(payload_b64.as_bytes());
    let sig_b64 = engine.encode(signature.to_bytes());

    Ok(format!("{}.{}", payload_b64, sig_b64))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn bytes_from_hex(hex: &str) -> Result<Vec<u8>, String> {
    if hex.len() % 2 != 0 {
        return Err("Hex string must have even length".into());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| format!("Invalid hex: {}", e)))
        .collect()
}
