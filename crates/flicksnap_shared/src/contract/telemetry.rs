//! The crash and error reporting contract: one project, one release name, one opt-out variable
//! (C#: `TelemetryContract`).

crate::contract! {
    /// The Sentry project's client-side DSN. Not a secret: it ships inside the application and only
    /// grants permission to submit events.
    pub const DSN: &str = "";
    /// Any non-empty value turns reporting off. Every process honours it, and every process the shell
    /// spawns inherits its environment, so opting out once covers all of them.
    pub const OPT_OUT_VARIABLE: &str = flicksnap_ipc::env::DISABLE_TELEMETRY;
    /// The release name, with the version as {0}: every process reports against one release.
    pub const RELEASE_FORMAT: &str = "flicksnap@{0}";
    /// The Sentry environment every process reports under.
    pub const ENVIRONMENT: &str = "production";
}
