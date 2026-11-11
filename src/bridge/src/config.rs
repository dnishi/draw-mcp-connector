//! Settings of the bridge. It has no UI, so every value is a constant here.

use std::time::Duration;

/// User-facing name (app name and menu bar tooltip).
/// The executable and settings directory are named after [[bin]] name in Cargo.toml (draw-mcp-connector)
pub const APP_NAME: &str = "Draw MCP Connector";

/// Port of the WebTransport (QUIC/UDP) server the canvas page connects to
pub const WEBTRANSPORT_PORT: u16 = 47100;
/// Port of the MCP server (Streamable HTTP, 127.0.0.1 only) AI agents connect to
pub const MCP_PORT: u16 = 47101;
pub const MCP_PATH: &str = "/mcp";
/// Host headers accepted by MCP (protection against DNS rebinding; it still listens only on 127.0.0.1).
/// host.docker.internal is the name used to reach the MCP server on the Mac from inside a Docker Desktop container
pub const MCP_ALLOWED_HOSTS: [&str; 4] = ["localhost", "127.0.0.1", "::1", "host.docker.internal"];
/// Name the MCP server identifies itself with. Also used in the registration command shown to the user
pub const MCP_SERVER_NAME: &str = "draw";

/// Canvas page. Replaced at build time with the BRIDGE_CANVAS_URL environment variable.
/// If unset or empty, the Vite dev server on the same PC (`npm run dev` in src/ui)
pub const CANVAS_URL: &str = match option_env!("BRIDGE_CANVAS_URL") {
    Some(url) if !url.is_empty() => url,
    _ => "http://127.0.0.1:5173/",
};

/// Validity of the self-signed certificate. WebTransport serverCertificateHashes requires 14 days or less in total
pub const CERTIFICATE_VALIDITY_DAYS: u32 = 14;
/// Interval for regenerating before expiry
pub const CERTIFICATE_RENEW_INTERVAL: Duration = Duration::from_secs(13 * 24 * 60 * 60);

/// Time limit for the canvas page to send its first message (hello) after connecting
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// Time limit when asking the user to approve or to scan the QR code
pub const USER_RESPONSE_TIMEOUT: Duration = Duration::from_secs(180);
/// How long a resume key (resume.rs) can be used. While connected, the key is replaced at this interval;
/// after disconnecting, the last key handed out stays usable for this long (reconnecting without the approval dialog)
pub const RESUME_KEY_LIFETIME: Duration = Duration::from_secs(60);
