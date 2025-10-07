use tracing_subscriber::{EnvFilter, prelude::*};

/// Initializes the global tracing subsystem with a filter based on the `RUST_LOG` environment variable or a default level of `info`.
/// Logs are formatted compactly with target and level included, and can be switched to `.pretty()` for human-readable multi-line output.
/// This function is called during startup to set up structured logging for the application.
///
/// # Notes
/// - The filter is derived from `RUST_LOG` if set; otherwise defaults to `info`.
/// - The `tracing_subscriber` is configured to emit logs with targets and levels, using a compact format by default.
/// - To enable verbose, multi-line logs, replace `.compact()` with `.pretty()` in the layer configuration.
/// - This function is private and intended to be called only by the application's initialization code.
fn init_tracing() {
    // Respect RUST_LOG if set; otherwise default to a sensible baseline.
    // Example: RUST_LOG=awful_rustdocs=debug,awful_aj=info,nu=warn
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap();

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_level(true)
                .compact(), // switch to .pretty() if you prefer multi-line human logs
        )
        .init();
}
